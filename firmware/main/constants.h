#pragma once

#include <stdint.h>

#include "sdkconfig.h"

// Every fixed value the firmware depends on, in one place. Values a builder is
// expected to change live in Kconfig.projbuild instead (CONFIG_FAN_*); these
// are the board wiring, the wire protocol and the internal sizing around them.

// ---- Units ----

#define US_PER_MS 1000
#define US_PER_S  1000000
#define S_PER_MIN 60

// A macro's value as a string literal, for building messages that state it.
#define FAN_STR_(x) #x
#define FAN_STR(x)  FAN_STR_(x)

// ---- Fan duty ----

#define FAN_DUTY_MAX_PERCENT 100

// ---- Fan PWM output ----

// GPIO3 -> 10k -> 2N3904 base; collector pulls the fan PWM line low, so the
// transistor stage inverts the signal. The LEDC channel's output_invert
// compensates: the duty written there is the duty the fan sees on its PWM pin.
#define FAN_PWM_GPIO            3
#define FAN_PWM_FREQ_HZ         25000  // Intel 4-wire fan spec: 21-28 kHz
#define FAN_PWM_RESOLUTION_BITS 10
// LEDC full-on value: one past the largest n-bit count.
#define FAN_PWM_MAX_DUTY        (1u << FAN_PWM_RESOLUTION_BITS)

// ---- Tachometer input ----

// Open-collector tach output with external 10k pull-up to 3.3V on GPIO4.
// The ESP32-C3 has no PCNT peripheral, so pulses are counted in a GPIO ISR;
// tach frequencies (< 200 Hz) make this trivial.
#define TACH_GPIO           4
#define TACH_PULSES_PER_REV 2
#define TACH_WINDOW_US      US_PER_S
// No ESP_INTR_FLAG_IRAM: see tach_isr() for what that costs.
#define TACH_ISR_FLAGS      0

// ---- Host protocol over USB Serial/JTAG (PROTOCOL.md) ----

// Hosts gate on the major version only, and ignore STATUS fields they do not
// know, so a minor version may add fields.
#define PROTOCOL_VERSION "1.0.0"

#define PROTO_CMD_PWM    "PWM"
#define PROTO_CMD_PING   "PING"
#define PROTO_CMD_STATUS "STATUS"
#define PROTO_CMD_VER    "VER"

#define PROTO_REPLY_OK   "OK"
#define PROTO_REPLY_PONG "PONG"

#define PROTO_ERR_EMPTY         "ERR empty command"
#define PROTO_ERR_PWM_ARGUMENT \
    "ERR " PROTO_CMD_PWM " expects an integer 0-" FAN_STR(FAN_DUTY_MAX_PERCENT)
#define PROTO_ERR_LINE_TOO_LONG "ERR line too long"
#define PROTO_ERR_UNKNOWN       "ERR unknown command"

#define PROTO_LINE_MAX  64
// Longest reply plus its newline and terminator. The STATUS line is the longest
// at 82 bytes with worst-case RPM, uptime and timeout.
#define PROTO_REPLY_MAX 96
// How long a reply may wait for room in the TX ring before it is dropped.
#define PROTO_WRITE_TIMEOUT_MS 100

#define PROTO_USB_RX_BUFFER_BYTES 256
#define PROTO_USB_TX_BUFFER_BYTES 512

#define PROTO_TASK_STACK_BYTES 4096
#define PROTO_TASK_PRIORITY    10

#define FAILSAFE_TIMEOUT_US ((int64_t)CONFIG_FAN_FAILSAFE_TIMEOUT_S * US_PER_S)

// ---- HTTP status page (PROTOCOL.md, "HTTP API") ----

#define HTTP_URI_ROOT   "/"
#define HTTP_URI_STATUS "/api/status"

#define HTTP_TYPE_HTML "text/html"
#define HTTP_TYPE_JSON "application/json"
// gzip rather than br/zstd: browsers only accept those over HTTPS, and this
// device serves plain HTTP on the LAN.
#define HTTP_HEADER_CONTENT_ENCODING "Content-Encoding"
#define HTTP_PAGE_ENCODING "gzip"

// Large enough for "-128" and for "null".
#define HTTP_RSSI_FIELD_MAX 16
#define HTTP_STATUS_BODY_MAX 160

// ---- WiFi and mDNS ----

#define WIFI_BACKOFF_INITIAL_MS 1000
#define WIFI_BACKOFF_MAX_MS     60000
#define WIFI_BACKOFF_FACTOR     2

// Low priority on purpose: the reconnect task must never compete with the
// protocol task or the esp_timer task, and it has nothing urgent to do.
#define WIFI_RECONNECT_TASK_STACK_BYTES 4096
#define WIFI_RECONNECT_TASK_PRIORITY    3

#define MDNS_INSTANCE_NAME "PC Fan Controller"
#define MDNS_SERVICE_TYPE  "_http"
#define MDNS_SERVICE_PROTO "_tcp"
#define MDNS_SERVICE_PORT  80
