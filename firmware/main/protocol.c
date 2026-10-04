#include "protocol.h"

#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "driver/usb_serial_jtag.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "sdkconfig.h"

#include "constants.h"
#include "failsafe.h"
#include "fan_pwm.h"
#include "fan_status.h"

// Line-based ASCII protocol over the native USB Serial/JTAG port. See
// PROTOCOL.md. The console/logging is routed to UART0 in sdkconfig.defaults,
// so this channel carries protocol traffic exclusively.

static const char *TAG = "protocol";

// One write, never two. usb_serial_jtag_write_bytes is all-or-nothing per call
// and returns 0 if the TX ring stays full for the timeout, so sending the body
// and the newline separately would let one succeed while the other is dropped:
// the host then reads a bare newline, or two replies merged into one line, and
// every subsequent request/response pair is misaligned. A whole dropped line is
// recoverable, since the host times out and reconnects, but half of one corrupts
// the framing silently.
static void reply(const char *line)
{
    char out[PROTO_REPLY_MAX];
    int len = snprintf(out, sizeof(out), "%s\n", line);
    if (len <= 0 || (size_t)len >= sizeof(out)) {
        return;  // unreachable for every reply this firmware produces
    }
    usb_serial_jtag_write_bytes(out, (size_t)len, pdMS_TO_TICKS(PROTO_WRITE_TIMEOUT_MS));
}

static void handle_line(char *line)
{
    if (line[0] == '\0') {
        // Every command gets exactly one reply, including this one: a host that
        // writes a stray newline would otherwise block until its own timeout.
        reply(PROTO_ERR_EMPTY);
        return;
    }

    // Matching "PWM" plus a terminator or space, rather than "PWM ", so a bare
    // PWM reports the argument error instead of falling through to "unknown
    // command". strtol skips the leading space itself.
    const size_t pwm_len = strlen(PROTO_CMD_PWM);
    if (strncmp(line, PROTO_CMD_PWM, pwm_len) == 0 &&
        (line[pwm_len] == '\0' || line[pwm_len] == ' ')) {
        const char *arg = line + pwm_len;
        char *end;
        long value = strtol(arg, &end, 10);
        if (end == arg || *end != '\0' || value < 0 || value > FAN_DUTY_MAX_PERCENT) {
            reply(PROTO_ERR_PWM_ARGUMENT);
            return;
        }
        failsafe_host_command((uint8_t)value);
        reply(PROTO_REPLY_OK);
    } else if (strcmp(line, PROTO_CMD_PING) == 0) {
        // Deliberately does NOT rearm the failsafe. The failsafe exists to catch
        // a host that has stopped controlling the fan, and a keepalive proves
        // only that the link is up -- a daemon whose temperature loop had died
        // could hold the fan at a low duty indefinitely just by pinging. PWM is
        // the only thing that counts as control.
        reply(PROTO_REPLY_PONG);
    } else if (strcmp(line, PROTO_CMD_STATUS) == 0) {
        // One byte short of the reply buffer, which reply() needs for the newline.
        char buf[PROTO_REPLY_MAX - 1];
        const fan_status_t status = fan_status_read();
        snprintf(buf, sizeof(buf),
                 PROTO_CMD_STATUS " PWM=%u RPM=%lu FAILSAFE=%d UPTIME=%lld FAILSAFE_TIMEOUT=%d",
                 status.pwm_percent, (unsigned long)status.rpm, status.failsafe ? 1 : 0,
                 status.uptime_s, CONFIG_FAN_FAILSAFE_TIMEOUT_S);
        reply(buf);
    } else if (strcmp(line, PROTO_CMD_VER) == 0) {
        reply(PROTO_CMD_VER " " PROTOCOL_VERSION);
    } else {
        reply(PROTO_ERR_UNKNOWN);
    }
}

static void protocol_task(void *arg)
{
    static char line[PROTO_LINE_MAX];
    size_t len = 0;
    bool overflow = false;

    for (;;) {
        uint8_t byte;
        if (usb_serial_jtag_read_bytes(&byte, 1, portMAX_DELAY) != 1) {
            continue;
        }
        if (byte == '\n') {
            if (overflow) {
                // Truncation must never be silent. Dropping the tail of
                // "PWM 0000...0" leaves "PWM 000...0", which parses as a
                // perfectly valid PWM 0, so an over-long line could change the
                // fan speed.
                reply(PROTO_ERR_LINE_TOO_LONG);
            } else {
                line[len] = '\0';
                handle_line(line);
            }
            len = 0;
            overflow = false;
        } else if (byte != '\r') {
            if (len < PROTO_LINE_MAX - 1) {
                line[len++] = (char)byte;
            } else {
                overflow = true;
            }
        }
    }
}

void protocol_start(void)
{
    usb_serial_jtag_driver_config_t usb_cfg = {
        .rx_buffer_size = PROTO_USB_RX_BUFFER_BYTES,
        .tx_buffer_size = PROTO_USB_TX_BUFFER_BYTES,
    };
    ESP_ERROR_CHECK(usb_serial_jtag_driver_install(&usb_cfg));

    if (xTaskCreate(protocol_task, "protocol", PROTO_TASK_STACK_BYTES, NULL, PROTO_TASK_PRIORITY,
                    NULL) != pdPASS) {
        // Without this task the host can never take control. Do now what the
        // failsafe would do at its timeout anyway. Rebooting instead would fail
        // the same way every time and hold the boot duty, which may be 50%.
        ESP_LOGE(TAG, "cannot create the protocol task; holding the fan at %d%%",
                 FAN_DUTY_MAX_PERCENT);
        fan_pwm_set_percent(FAN_DUTY_MAX_PERCENT);
    }
}
