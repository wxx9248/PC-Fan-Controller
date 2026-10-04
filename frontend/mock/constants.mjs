// Every fixed value the mock controller depends on, in one place: the model of
// the real fan, and the timings of the simulation that drives it.

// ---- Serving ----

export const MOCK_HOST = "127.0.0.1";
// The standalone default. Under `pnpm dev` the port comes from VITE_API_BASE in
// .env.development instead, which uses the same number.
export const DEFAULT_MOCK_PORT = 8787;

export const HTTP_OK = 200;
export const HTTP_NOT_FOUND = 404;
export const HTTP_METHOD_NOT_ALLOWED = 405;

// ---- The real fan, measured on hardware (5%-step sweep, 2026-09-29) ----
//
//   RPM = 15.76 * duty + 397   for duty >= 32%   (R^2 = 0.997)
//   RPM = 840 (floor)          for duty <= 31%, including 0 -- it never stops
//
// The firmware's tachometer counts pulses over a 1 s window at 2 pulses/rev, so
// every RPM it can report is a multiple of 30.
//
// A plant like `350 + pwm*24` has no floor, peaks near 2750, and passes
// through zero, so a chart or a formatter tuned against it is tuned
// against a fan that does not exist.
export const FLOOR_RPM = 840;
export const TACH_QUANTUM_RPM = 30;
// The knee is in DUTY, not in RPM: the fan holds its floor all the way to 31%
// and then steps onto the linear region at 32%. Clamping the line with
// max(FLOOR, line) instead would report 886 RPM at 31%, where the hardware
// measures 840.
export const KNEE_DUTY = 32;
export const RPM_PER_DUTY_PERCENT = 15.76;
export const RPM_AT_ZERO_DUTY = 397;

// ---- Simulation ----

// The firmware boots at 100% with its failsafe flag set, and holds both until
// the first PWM command (PROTOCOL.md).
export const BOOT_DUTY_PERCENT = 100;
export const BOOT_FAILSAFE_MS = 8000;
// Where the simulated daemon first steers the fan.
export const INITIAL_TARGET_DUTY = 31;
export const INITIAL_RSSI_DBM = -52;

export const TICK_MS = 1000;
export const RETARGET_MS = 15000;
// Each retarget rolls once: below the first threshold a failsafe episode,
// below the second a WiFi dropout, otherwise a new duty.
export const FAILSAFE_CHANCE = 0.1;
export const WIFI_DOWN_CHANCE = 0.2;
export const FAILSAFE_EPISODE_MS = 12000;
export const WIFI_DOWN_MS = 10000;
// New targets are drawn uniformly from this range.
export const TARGET_DUTY_MIN = 28;
export const TARGET_DUTY_SPAN = 60;

// How far the simulated daemon moves the duty per tick, and the fraction of
// the remaining gap the fan's speed closes per tick.
export const DUTY_SLEW_PER_TICK = 4;
export const RPM_RESPONSE_PER_TICK = 0.3;

// RSSI random walk: each tick moves it by up to half this either way, and it
// stays within the band below.
export const RSSI_JITTER_DBM = 2;
export const RSSI_MIN_DBM = -75;
export const RSSI_MAX_DBM = -40;
