// The /api/status contract from PROTOCOL.md ("HTTP API"): a flat object with
// exactly these five fields. The page checks every reply against it, and the
// mock's tests check the mock against the same definition, so the two cannot
// disagree about what a valid reply is.

import { MAX_DUTY_PERCENT } from "./constants.js";

/** A reply that arrived but breaks the contract. */
export class StatusContractError extends Error {}

export const STATUS_FIELDS = ["pwmPercent", "rpm", "failsafe", "uptimeSeconds", "wifiRssiDbm"];

/**
 * Validate a decoded /api/status body and return just the contract's fields.
 * Throws StatusContractError naming the first field that is missing or wrong.
 */
export function parseStatus(body) {
  if (body === null || typeof body !== "object" || Array.isArray(body)) {
    throw new StatusContractError("the reply is not a JSON object");
  }
  const { pwmPercent, rpm, failsafe, uptimeSeconds, wifiRssiDbm } = body;
  check("pwmPercent", pwmPercent, Number.isInteger(pwmPercent) && pwmPercent >= 0 && pwmPercent <= MAX_DUTY_PERCENT);
  check("rpm", rpm, Number.isInteger(rpm) && rpm >= 0);
  check("failsafe", failsafe, typeof failsafe === "boolean");
  check("uptimeSeconds", uptimeSeconds, Number.isInteger(uptimeSeconds) && uptimeSeconds >= 0);
  check("wifiRssiDbm", wifiRssiDbm, wifiRssiDbm === null || Number.isInteger(wifiRssiDbm));
  return { pwmPercent, rpm, failsafe, uptimeSeconds, wifiRssiDbm };
}

function check(field, value, ok) {
  if (!ok) {
    throw new StatusContractError(`${field} is ${JSON.stringify(value) ?? "missing"}`);
  }
}
