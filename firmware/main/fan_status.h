#pragma once

#include <stdbool.h>
#include <stdint.h>

// The state both status interfaces report: STATUS over USB and /api/status
// over HTTP. Read in one place, so the two cannot drift apart.
typedef struct {
    uint8_t pwm_percent;
    uint32_t rpm;
    bool failsafe;
    int64_t uptime_s;
} fan_status_t;

fan_status_t fan_status_read(void);
