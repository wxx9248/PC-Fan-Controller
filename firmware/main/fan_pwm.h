#pragma once

#include <stdint.h>

void fan_pwm_init(uint8_t boot_percent);
void fan_pwm_set_percent(uint8_t percent);
uint8_t fan_pwm_get_percent(void);
