#pragma once

#include <stdbool.h>
#include <stdint.h>

// The fan's protection against a host that stops controlling it: unless a PWM
// command arrives at least every CONFIG_FAN_FAILSAFE_TIMEOUT_S, the fan is
// forced to 100% until one does.

// Create the timer and arm it at once, so a host that never speaks still gets
// the fan to 100% at the timeout, whatever the boot duty.
void failsafe_start(void);
// Apply a duty the host commanded and restart the countdown. Only this counts
// as host control.
void failsafe_host_command(uint8_t percent);
// True whenever the host is not in control: from boot until the first PWM
// command, and again whenever the timer expires.
bool failsafe_active(void);
