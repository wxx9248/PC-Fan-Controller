#include "http_server.h"

#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "esp_http_server.h"
#include "esp_log.h"

#include "constants.h"
#include "fan_status.h"
#include "wifi_mgr.h"

// Read-only status endpoints. Control stays exclusively with the USB host.

static const char *TAG = "http_server";

// Svelte frontend, built by `pnpm build` in frontend/ and embedded gzipped.
extern const uint8_t index_html_gz_start[] asm("_binary_index_html_gz_start");
extern const uint8_t index_html_gz_end[] asm("_binary_index_html_gz_end");

static esp_err_t root_get_handler(httpd_req_t *req)
{
    httpd_resp_set_type(req, HTTP_TYPE_HTML);
    httpd_resp_set_hdr(req, HTTP_HEADER_CONTENT_ENCODING, HTTP_PAGE_ENCODING);
    // The start and end symbols are two separate objects as far as C is
    // concerned, so subtracting the pointers is undefined; their addresses are
    // just numbers, and subtracting those is not.
    const size_t size = (size_t)((uintptr_t)index_html_gz_end - (uintptr_t)index_html_gz_start);
    return httpd_resp_send(req, (const char *)index_html_gz_start, (ssize_t)size);
}

static esp_err_t status_get_handler(httpd_req_t *req)
{
    char rssi_field[HTTP_RSSI_FIELD_MAX];
    int8_t rssi;
    if (wifi_mgr_get_rssi(&rssi)) {
        snprintf(rssi_field, sizeof(rssi_field), "%d", rssi);
    } else {
        strcpy(rssi_field, "null");
    }

    char buf[HTTP_STATUS_BODY_MAX];
    const fan_status_t status = fan_status_read();
    snprintf(buf, sizeof(buf),
             "{\"pwmPercent\":%u,\"rpm\":%lu,\"failsafe\":%s,"
             "\"uptimeSeconds\":%lld,\"wifiRssiDbm\":%s}",
             status.pwm_percent, (unsigned long)status.rpm, status.failsafe ? "true" : "false",
             status.uptime_s, rssi_field);

    httpd_resp_set_type(req, HTTP_TYPE_JSON);
    return httpd_resp_send(req, buf, HTTPD_RESP_USE_STRLEN);
}

static httpd_handle_t s_server;

void http_server_start(void)
{
    httpd_config_t config = HTTPD_DEFAULT_CONFIG();
    // The default allows 7 client sockets, on top of the 3 the server keeps for
    // itself. Browsers routinely hold several connections per origin, so without
    // LRU purging a few open tabs exhaust the table and the server refuses new
    // connections instead of evicting the oldest. The page then reports
    // "unreachable" while the device is perfectly healthy.
    config.lru_purge_enable = true;
    if (httpd_start(&s_server, &config) != ESP_OK) {
        ESP_LOGE(TAG, "failed to start HTTP server");
        return;
    }

    const httpd_uri_t root_uri = {
        .uri = HTTP_URI_ROOT,
        .method = HTTP_GET,
        .handler = root_get_handler,
    };
    const httpd_uri_t status_uri = {
        .uri = HTTP_URI_STATUS,
        .method = HTTP_GET,
        .handler = status_get_handler,
    };
    // An unchecked failure here would leave a server that 404s everything with
    // no indication why.
    if (httpd_register_uri_handler(s_server, &root_uri) != ESP_OK ||
        httpd_register_uri_handler(s_server, &status_uri) != ESP_OK) {
        ESP_LOGE(TAG, "failed to register HTTP handlers");
    }
}
