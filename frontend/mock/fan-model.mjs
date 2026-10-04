// The real fan as the controller reports it; see constants.mjs for the
// measurements behind these numbers.
import { FLOOR_RPM, KNEE_DUTY, RPM_AT_ZERO_DUTY, RPM_PER_DUTY_PERCENT, TACH_QUANTUM_RPM } from "./constants.mjs";

export function rpmForDuty(duty) {
  if (duty < KNEE_DUTY) return FLOOR_RPM;
  return RPM_PER_DUTY_PERCENT * duty + RPM_AT_ZERO_DUTY;
}

export function quantiseRpm(rpm) {
  return Math.round(rpm / TACH_QUANTUM_RPM) * TACH_QUANTUM_RPM;
}
