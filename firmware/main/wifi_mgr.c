#include "wifi_mgr.h"

#include <stdatomic.h>
#include <string.h>

#include "esp_event.h"
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"
#include "freertos/task.h"
#include "esp_log.h"
#include "esp_netif.h"
#include "esp_timer.h"
#include "esp_wifi.h"
#include "mdns.h"
#include "sdkconfig.h"

#include "constants.h"

// WiFi exists only for the read-only HTTP status page and must never affect fan
// control. So initialisation failure here logs and disables networking instead of
// aborting, and the disconnect path retries with capped backoff. Do not reach for
// ESP_ERROR_CHECK in this file: it calls abort(), which turns an mDNS or WiFi
// init failure into a reboot of the fan controller.

static const char *TAG = "wifi_mgr";

static esp_timer_handle_t s_reconnect_timer;
static SemaphoreHandle_t s_reconnect_signal;
static TaskHandle_t s_reconnect_task;
// Touched only by the event task, so it needs no synchronisation.
static uint32_t s_backoff_ms = WIFI_BACKOFF_INITIAL_MS;
// Written by the event task, read by the httpd task.
static _Atomic bool s_connected;

// esp_timer dispatches every TASK-method callback from one shared task -- the
// same task that fires the fan failsafe. esp_wifi_connect() blocks on the WiFi
// API lock with no timeout, so calling it here would let a stalled WiFi stack
// delay, or indefinitely prevent, the failsafe forcing the fan to 100%. The
// callback therefore only signals; the blocking call happens on its own task.
static void reconnect_cb(void *arg)
{
    xSemaphoreGive(s_reconnect_signal);
}

// Runs at WIFI_RECONNECT_TASK_PRIORITY.
static void reconnect_task(void *arg)
{
    for (;;) {
        xSemaphoreTake(s_reconnect_signal, portMAX_DELAY);
        // Retries are scheduled from the DISCONNECTED event, and an attempt the
        // driver refuses outright produces no such event, so without this the
        // retries would stop for good. Fails if a retry is already pending.
        if (esp_wifi_connect() != ESP_OK) {
            esp_timer_start_once(s_reconnect_timer, (uint64_t)WIFI_BACKOFF_MAX_MS * US_PER_MS);
        }
    }
}

// The reconnect timer, its signal and the task that acts on it. Created only
// once nothing else can fail before the radio starts, and removed again if it
// does not start, so a WiFi failure leaves nothing behind.
static bool reconnect_create(void)
{
    const esp_timer_create_args_t timer_args = {
        .callback = reconnect_cb,
        .name = "wifi_reconnect",
    };
    s_reconnect_signal = xSemaphoreCreateBinary();
    if (s_reconnect_signal == NULL) {
        return false;
    }
    if (esp_timer_create(&timer_args, &s_reconnect_timer) != ESP_OK) {
        vSemaphoreDelete(s_reconnect_signal);
        return false;
    }
    if (xTaskCreate(reconnect_task, "wifi_reconnect", WIFI_RECONNECT_TASK_STACK_BYTES, NULL,
                    WIFI_RECONNECT_TASK_PRIORITY, &s_reconnect_task) != pdPASS) {
        esp_timer_delete(s_reconnect_timer);
        vSemaphoreDelete(s_reconnect_signal);
        return false;
    }
    return true;
}

static void reconnect_destroy(void)
{
    vTaskDelete(s_reconnect_task);
    esp_timer_stop(s_reconnect_timer);
    esp_timer_delete(s_reconnect_timer);
    vSemaphoreDelete(s_reconnect_signal);
}

static void event_handler(void *arg, esp_event_base_t base, int32_t id, void *data)
{
    if (base == WIFI_EVENT && id == WIFI_EVENT_STA_START) {
        esp_wifi_connect();
    } else if (base == WIFI_EVENT && id == WIFI_EVENT_STA_DISCONNECTED) {
        atomic_store(&s_connected, false);
        ESP_LOGW(TAG, "disconnected, retrying in %lu ms", (unsigned long)s_backoff_ms);
        esp_timer_stop(s_reconnect_timer);
        esp_timer_start_once(s_reconnect_timer, (uint64_t)s_backoff_ms * US_PER_MS);
        // Exponential backoff, capped so a long outage keeps a steady retry
        // cadence instead of growing without bound.
        s_backoff_ms *= WIFI_BACKOFF_FACTOR;
        if (s_backoff_ms > WIFI_BACKOFF_MAX_MS) {
            s_backoff_ms = WIFI_BACKOFF_MAX_MS;
        }
    } else if (base == IP_EVENT && id == IP_EVENT_STA_GOT_IP) {
        const ip_event_got_ip_t *event = data;
        atomic_store(&s_connected, true);
        s_backoff_ms = WIFI_BACKOFF_INITIAL_MS;
        ESP_LOGI(TAG, "got IP " IPSTR ", http://%s.local",
                 IP2STR(&event->ip_info.ip), CONFIG_FAN_HOSTNAME);
    }
}

// Failure here costs only the .local name; WiFi and HTTP still work, so it is
// reported and shrugged off rather than being allowed to take the fan down.
static void mdns_start(void)
{
    esp_err_t err = mdns_init();
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "mdns_init failed: %s; no .local name", esp_err_to_name(err));
        return;
    }
    if (mdns_hostname_set(CONFIG_FAN_HOSTNAME) != ESP_OK ||
        mdns_instance_name_set(MDNS_INSTANCE_NAME) != ESP_OK ||
        mdns_service_add(NULL, MDNS_SERVICE_TYPE, MDNS_SERVICE_PROTO, MDNS_SERVICE_PORT, NULL,
                         0) != ESP_OK) {
        ESP_LOGE(TAG, "mDNS advertisement incomplete; no .local name");
    }
}

// Report and disable networking instead of aborting. The fan controller must
// survive anything that goes wrong out here.
#define TRY(expr, what)                                                       \
    do {                                                                      \
        esp_err_t _err = (expr);                                              \
        if (_err != ESP_OK) {                                                 \
            ESP_LOGE(TAG, "%s failed: %s; WiFi disabled", (what),             \
                     esp_err_to_name(_err));                                  \
            return false;                                                     \
        }                                                                     \
    } while (0)

bool wifi_mgr_start(void)
{
    if (strlen(CONFIG_FAN_WIFI_SSID) == 0) {
        ESP_LOGW(TAG, "no WiFi SSID configured (sdkconfig.secrets), WiFi disabled");
        return false;
    }

    TRY(esp_netif_init(), "esp_netif_init");
    TRY(esp_event_loop_create_default(), "esp_event_loop_create_default");
    // What esp_netif_create_default_wifi_sta() does, minus its assert() and
    // ESP_ERROR_CHECK()s: a failure in it would reboot the fan controller.
    esp_netif_config_t netif_cfg = ESP_NETIF_DEFAULT_WIFI_STA();
    esp_netif_t *netif = esp_netif_new(&netif_cfg);
    TRY(netif ? ESP_OK : ESP_ERR_NO_MEM, "esp_netif_new");
    TRY(esp_netif_attach_wifi_station(netif), "esp_netif_attach_wifi_station");
    TRY(esp_wifi_set_default_wifi_sta_handlers(), "esp_wifi_set_default_wifi_sta_handlers");
    if (esp_netif_set_hostname(netif, CONFIG_FAN_HOSTNAME) != ESP_OK) {
        ESP_LOGW(TAG, "cannot set the DHCP hostname; continuing without it");
    }

    wifi_init_config_t init_cfg = WIFI_INIT_CONFIG_DEFAULT();
    TRY(esp_wifi_init(&init_cfg), "esp_wifi_init");
    TRY(esp_event_handler_register(WIFI_EVENT, ESP_EVENT_ANY_ID, event_handler, NULL),
        "registering the WiFi event handler");
    TRY(esp_event_handler_register(IP_EVENT, IP_EVENT_STA_GOT_IP, event_handler, NULL),
        "registering the IP event handler");

    wifi_config_t wifi_cfg = {
        .sta = {
            .threshold.authmode = strlen(CONFIG_FAN_WIFI_PASSWORD) > 0
                                      ? WIFI_AUTH_WPA2_PSK
                                      : WIFI_AUTH_OPEN,
        },
    };
    // memcpy, not strlcpy: these fields are fixed-width byte arrays that do not
    // require a terminator when full, whereas strlcpy always reserves one byte
    // for it. A 32-character SSID (the 802.11 maximum) or a 64-character raw
    // WPA2 PSK would silently lose its last character and never associate, with
    // nothing to distinguish that from a wrong password.
    const size_t ssid_len = strlen(CONFIG_FAN_WIFI_SSID);
    const size_t pass_len = strlen(CONFIG_FAN_WIFI_PASSWORD);
    if (ssid_len > sizeof(wifi_cfg.sta.ssid) ||
        pass_len > sizeof(wifi_cfg.sta.password)) {
        ESP_LOGE(TAG, "WiFi SSID or password too long; WiFi disabled");
        return false;
    }
    memcpy(wifi_cfg.sta.ssid, CONFIG_FAN_WIFI_SSID, ssid_len);
    memcpy(wifi_cfg.sta.password, CONFIG_FAN_WIFI_PASSWORD, pass_len);

    TRY(esp_wifi_set_mode(WIFI_MODE_STA), "esp_wifi_set_mode");
    TRY(esp_wifi_set_config(WIFI_IF_STA, &wifi_cfg), "esp_wifi_set_config");

    // No event reaches the handler before esp_wifi_start(), so this is the last
    // moment the reconnect machinery can be created and the first it is needed.
    if (!reconnect_create()) {
        ESP_LOGE(TAG, "cannot create the reconnect timer or task; WiFi disabled");
        return false;
    }
    esp_err_t err = esp_wifi_start();
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "esp_wifi_start failed: %s; WiFi disabled", esp_err_to_name(err));
        esp_event_handler_unregister(WIFI_EVENT, ESP_EVENT_ANY_ID, event_handler);
        esp_event_handler_unregister(IP_EVENT, IP_EVENT_STA_GOT_IP, event_handler);
        reconnect_destroy();
        return false;
    }

    mdns_start();
    return true;
}

bool wifi_mgr_get_rssi(int8_t *rssi_dbm)
{
    if (!atomic_load(&s_connected)) {
        return false;
    }
    wifi_ap_record_t ap;
    if (esp_wifi_sta_get_ap_info(&ap) != ESP_OK) {
        return false;
    }
    *rssi_dbm = ap.rssi;
    return true;
}
