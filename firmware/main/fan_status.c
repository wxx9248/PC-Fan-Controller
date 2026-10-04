#include "fan_status.h"

#include "esp_timer.h"

#include "constants.h"
#include "failsafe.h"
#include "fan_pwm.h"
#include "tach.h"

fan_status_t fan_status_read(void)
{
    return (fan_status_t){
        .pwm_percent = fan_pwm_get_percent(),
        .rpm = tach_get_rpm(),
        .failsafe = failsafe_active(),
        .uptime_s = esp_timer_get_time() / US_PER_S,
    };
}
