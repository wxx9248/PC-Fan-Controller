// Simulated plant: the "daemon" retargets PWM as if drive temps change, RPM
// follows with inertia and jitter, and an occasional failsafe episode exercises
// the badge in the UI.
import { MAX_DUTY_PERCENT, MS_PER_S } from "../src/lib/constants.js";
import {
  BOOT_DUTY_PERCENT,
  BOOT_FAILSAFE_MS,
  DUTY_SLEW_PER_TICK,
  FAILSAFE_CHANCE,
  FAILSAFE_EPISODE_MS,
  INITIAL_RSSI_DBM,
  INITIAL_TARGET_DUTY,
  RETARGET_MS,
  RPM_RESPONSE_PER_TICK,
  RSSI_JITTER_DBM,
  RSSI_MAX_DBM,
  RSSI_MIN_DBM,
  TARGET_DUTY_MIN,
  TARGET_DUTY_SPAN,
  TICK_MS,
  WIFI_DOWN_CHANCE,
  WIFI_DOWN_MS,
} from "./constants.mjs";
import { quantiseRpm, rpmForDuty } from "./fan-model.mjs";

const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));

// Starts running at once. `status()` is the /api/status body as the firmware
// would send it now; `stop()` ends the timers.
export function createSimulation() {
  const startedAt = Date.now();

  let pwm = BOOT_DUTY_PERCENT;
  let targetPwm = INITIAL_TARGET_DUTY;
  let rpm = rpmForDuty(pwm);
  let rssi = INITIAL_RSSI_DBM;
  // A freshly powered device shows FAILSAFE immediately. Starting clear and
  // showing it only on a random roll would leave the badge's most important
  // state effectively untested.
  let failsafeUntil = Date.now() + BOOT_FAILSAFE_MS;
  // The firmware reports null RSSI whenever it is not associated. Without that
  // here, the page's placeholder branch is unreachable in development.
  let wifiDownUntil = 0;

  const retarget = setInterval(() => {
    const roll = Math.random();
    if (roll < FAILSAFE_CHANCE) {
      failsafeUntil = Date.now() + FAILSAFE_EPISODE_MS;
    } else if (roll < WIFI_DOWN_CHANCE) {
      wifiDownUntil = Date.now() + WIFI_DOWN_MS; // exercise the null-RSSI branch
    } else {
      targetPwm = Math.round(TARGET_DUTY_MIN + Math.random() * TARGET_DUTY_SPAN);
    }
  }, RETARGET_MS);

  const tick = setInterval(() => {
    const failsafe = Date.now() < failsafeUntil;
    const step = clamp(targetPwm - pwm, -DUTY_SLEW_PER_TICK, DUTY_SLEW_PER_TICK);
    // The firmware sets 100% in one atomic step rather than slewing to it.
    pwm = failsafe ? MAX_DUTY_PERCENT : clamp(pwm + step, 0, MAX_DUTY_PERCENT);
    rpm += (rpmForDuty(pwm) - rpm) * RPM_RESPONSE_PER_TICK;
    rssi = clamp(rssi + (Math.random() - 0.5) * RSSI_JITTER_DBM, RSSI_MIN_DBM, RSSI_MAX_DBM);
  }, TICK_MS);

  return {
    status: () => ({
      pwmPercent: Math.round(pwm),
      rpm: quantiseRpm(rpm),
      failsafe: Date.now() < failsafeUntil,
      uptimeSeconds: Math.floor((Date.now() - startedAt) / MS_PER_S),
      wifiRssiDbm: Date.now() < wifiDownUntil ? null : Math.round(rssi),
    }),
    stop: () => {
      clearInterval(retarget);
      clearInterval(tick);
    },
  };
}
