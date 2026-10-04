// Fetching /api/status from the controller.
import { DEFAULT_API_BASE, REQUEST_TIMEOUT_MS, STATUS_RESOURCE } from "./constants.js";
import { parseStatus, StatusContractError } from "./status.js";

const API_BASE = import.meta.env.VITE_API_BASE ?? DEFAULT_API_BASE;

/**
 * One validated status reading. Throws StatusContractError for a reply that
 * arrived but is unusable, and anything else for no reply at all.
 */
export async function fetchStatus() {
  const res = await fetch(`${API_BASE}${STATUS_RESOURCE}`, {
    signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
  });
  if (!res.ok) throw new StatusContractError(`HTTP ${res.status}`);
  let body;
  try {
    body = await res.json();
  } catch {
    throw new StatusContractError("the reply is not JSON");
  }
  // Checked before anything is drawn: one out-of-range value would otherwise
  // rescale the chart for the whole ten-minute window.
  return parseStatus(body);
}
