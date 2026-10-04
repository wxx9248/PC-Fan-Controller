#include "tach.h"

#include <stdatomic.h>

#include "driver/gpio.h"
#include "esp_timer.h"

#include "constants.h"

static _Atomic uint32_t s_pulses;
static _Atomic uint32_t s_rpm;
static esp_timer_handle_t s_window_timer;

// Not IRAM_ATTR: the ISR service is installed without ESP_INTR_FLAG_IRAM, so
// this interrupt is masked during flash writes wherever the handler lives, and
// placing it in IRAM changed nothing but its address. Pulses arriving during a
// flash write (rare: NVS commits) are lost, which shortens one window's count;
// the daemon's consecutive-cycle stall rule absorbs a single low reading.
static void tach_isr(void *arg)
{
    atomic_fetch_add(&s_pulses, 1);
}

static void tach_window_cb(void *arg)
{
    uint32_t pulses = atomic_exchange(&s_pulses, 0);
    // Scaled by the window rather than assuming it is one second. 64-bit,
    // because pulses times microseconds per minute overflows 32 bits.
    uint64_t per_minute = (uint64_t)pulses * S_PER_MIN * US_PER_S / TACH_WINDOW_US;
    atomic_store(&s_rpm, (uint32_t)(per_minute / TACH_PULSES_PER_REV));
}

void tach_init(void)
{
    const gpio_config_t cfg = {
        .pin_bit_mask = 1ULL << TACH_GPIO,
        .mode = GPIO_MODE_INPUT,
        .pull_up_en = GPIO_PULLUP_DISABLE,  // external 10k pull-up present
        .pull_down_en = GPIO_PULLDOWN_DISABLE,
        .intr_type = GPIO_INTR_NEGEDGE,
    };
    ESP_ERROR_CHECK(gpio_config(&cfg));
    ESP_ERROR_CHECK(gpio_install_isr_service(TACH_ISR_FLAGS));
    ESP_ERROR_CHECK(gpio_isr_handler_add(TACH_GPIO, tach_isr, NULL));

    const esp_timer_create_args_t timer_args = {
        .callback = tach_window_cb,
        .name = "tach_window",
    };
    // Retained rather than dropped on the stack: esp_timer_delete() requires a
    // stopped timer, so discarding the handle makes the timer unstoppable for
    // the lifetime of the process.
    ESP_ERROR_CHECK(esp_timer_create(&timer_args, &s_window_timer));
    ESP_ERROR_CHECK(esp_timer_start_periodic(s_window_timer, TACH_WINDOW_US));
}

uint32_t tach_get_rpm(void)
{
    return atomic_load(&s_rpm);
}
