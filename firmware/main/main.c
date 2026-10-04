#include "esp_log.h"
#include "nvs_flash.h"
#include "sdkconfig.h"

#include "failsafe.h"
#include "fan_pwm.h"
#include "http_server.h"
#include "protocol.h"
#include "tach.h"
#include "wifi_mgr.h"

static const char *TAG = "main";

void app_main(void)
{
    // Fan control first: the drives are protected before anything that can
    // fail (WiFi, HTTP) is even attempted.
    fan_pwm_init(CONFIG_FAN_BOOT_DUTY_PERCENT);
    tach_init();
    failsafe_start();
    protocol_start();

    // NVS holds WiFi calibration data, so a failure here costs networking at
    // worst. Ignoring any error other than the two recoverable ones, or wrapping
    // the erase/retry in ESP_ERROR_CHECK, would let a worn or mis-sized partition
    // abort the fan controller.
    esp_err_t err = nvs_flash_init();
    if (err == ESP_ERR_NVS_NO_FREE_PAGES || err == ESP_ERR_NVS_NEW_VERSION_FOUND) {
        if (nvs_flash_erase() == ESP_OK) {
            err = nvs_flash_init();
        }
    }
    if (err != ESP_OK) {
        ESP_LOGW(TAG, "NVS unavailable (%s); WiFi may not start",
                 esp_err_to_name(err));
    }

    // Only start the HTTP server if WiFi actually came up: without it the TCP/IP
    // stack is uninitialised and httpd_start() would panic rather than fail.
    if (wifi_mgr_start()) {
        http_server_start();
    } else {
        ESP_LOGW(TAG, "no network: HTTP status page not started (USB control unaffected)");
    }

    ESP_LOGI(TAG, "fan controller up, boot duty %d%%", CONFIG_FAN_BOOT_DUTY_PERCENT);
}
