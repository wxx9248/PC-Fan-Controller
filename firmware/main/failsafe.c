#include "failsafe.h"

#include <stdatomic.h>

#include "esp_err.h"
#include "esp_timer.h"

#include "constants.h"
#include "fan_pwm.h"

static _Atomic bool s_failsafe = true;
static esp_timer_handle_t s_failsafe_timer;

// Runs on the shared esp_timer task, so it must not block. The duty write takes
// a mutex held only across two LEDC register writes, which priority inheritance
// bounds. Logging is deliberately absent: ESP_LOGW would take the shared stdio
// lock -- contended by the lwIP and WiFi tasks -- and then busy-wait on the UART0
// FIFO for milliseconds, putting an unbounded delay in front of the one action
// that must never be delayed. Nothing is lost: UART0 is unconnected on this
// board, and the state is reported as FAILSAFE=1 by STATUS and /api/status.
static void failsafe_expired_cb(void *arg)
{
    atomic_store(&s_failsafe, true);
    fan_pwm_set_percent(FAN_DUTY_MAX_PERCENT);
}

void failsafe_start(void)
{
    const esp_timer_create_args_t timer_args = {
        .callback = failsafe_expired_cb,
        .name = "failsafe",
    };
    ESP_ERROR_CHECK(esp_timer_create(&timer_args, &s_failsafe_timer));
    // The flag is set from boot, and STATUS and /api/status report it.
    ESP_ERROR_CHECK(esp_timer_start_once(s_failsafe_timer, FAILSAFE_TIMEOUT_US));
}

void failsafe_host_command(uint8_t percent)
{
    // Stop the timer before anything else. Its callback runs on the
    // esp_timer task, which outranks the caller, so on this single-core chip it
    // has either already run by the time esp_timer_stop() returns or cannot
    // run until the timer is armed again. Clearing the flag before stopping
    // the timer would leave a window for an expiry to set it again, and
    // STATUS would report FAILSAFE=1 with the host's duty applied until the
    // next command.
    esp_timer_stop(s_failsafe_timer);
    fan_pwm_set_percent(percent);
    atomic_store(&s_failsafe, false);
    ESP_ERROR_CHECK(esp_timer_start_once(s_failsafe_timer, FAILSAFE_TIMEOUT_US));
}

bool failsafe_active(void)
{
    return atomic_load(&s_failsafe);
}
