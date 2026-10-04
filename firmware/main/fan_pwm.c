#include "fan_pwm.h"

#include <stdatomic.h>

#include "driver/ledc.h"
#include "esp_err.h"
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"

#include "constants.h"

static SemaphoreHandle_t s_lock;
// Written under s_lock, but read without it by the protocol and HTTP tasks, so
// it has to be atomic: `volatile` alone leaves that a data race in C11.
static _Atomic uint8_t s_percent;

static uint32_t percent_to_duty(uint8_t percent)
{
    if (percent >= FAN_DUTY_MAX_PERCENT) {
        return FAN_PWM_MAX_DUTY;
    }
    // Rounded to the nearest count rather than truncated.
    return ((uint32_t)percent * (FAN_PWM_MAX_DUTY - 1) + FAN_DUTY_MAX_PERCENT / 2) /
           FAN_DUTY_MAX_PERCENT;
}

void fan_pwm_init(uint8_t boot_percent)
{
    s_lock = xSemaphoreCreateMutex();
    // Aborting is the right response for this one allocation: without the lock
    // there is no safe way to drive the fan at all, and a reboot lands back at
    // the 100% boot duty. Everything that can fail later -- WiFi, mDNS, HTTP --
    // is handled gracefully instead.
    ESP_ERROR_CHECK(s_lock ? ESP_OK : ESP_ERR_NO_MEM);

    const ledc_timer_config_t timer_cfg = {
        .speed_mode = LEDC_LOW_SPEED_MODE,
        // LEDC_TIMER_n_BIT is n.
        .duty_resolution = (ledc_timer_bit_t)FAN_PWM_RESOLUTION_BITS,
        .timer_num = LEDC_TIMER_0,
        .freq_hz = FAN_PWM_FREQ_HZ,
        .clk_cfg = LEDC_AUTO_CLK,
    };
    ESP_ERROR_CHECK(ledc_timer_config(&timer_cfg));

    if (boot_percent > FAN_DUTY_MAX_PERCENT) {
        boot_percent = FAN_DUTY_MAX_PERCENT;
    }
    const ledc_channel_config_t channel_cfg = {
        .gpio_num = FAN_PWM_GPIO,
        .speed_mode = LEDC_LOW_SPEED_MODE,
        .channel = LEDC_CHANNEL_0,
        .intr_type = LEDC_INTR_DISABLE,
        .timer_sel = LEDC_TIMER_0,
        .duty = percent_to_duty(boot_percent),
        .hpoint = 0,
        .flags.output_invert = 1,
    };
    ESP_ERROR_CHECK(ledc_channel_config(&channel_cfg));
    atomic_store(&s_percent, boot_percent);
}

void fan_pwm_set_percent(uint8_t percent)
{
    if (percent > FAN_DUTY_MAX_PERCENT) {
        percent = FAN_DUTY_MAX_PERCENT;
    }
    xSemaphoreTake(s_lock, portMAX_DELAY);
    esp_err_t err = ledc_set_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_0,
                                  percent_to_duty(percent));
    if (err == ESP_OK) {
        err = ledc_update_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_0);
    }
    // Only claim the duty the hardware actually took. Updating this
    // unconditionally would let STATUS and /api/status report a fan speed that
    // was never applied -- including reporting a successful failsafe that did
    // not happen.
    if (err == ESP_OK) {
        atomic_store(&s_percent, percent);
    }
    xSemaphoreGive(s_lock);
}

uint8_t fan_pwm_get_percent(void)
{
    return atomic_load(&s_percent);
}
