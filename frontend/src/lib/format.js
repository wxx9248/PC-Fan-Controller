// How values are shown on the page.
import { FRESH_AGE_S, MS_PER_S, S_PER_DAY, S_PER_HOUR, S_PER_MIN } from "./constants.js";

export function formatUptime(s) {
  const d = Math.floor(s / S_PER_DAY);
  const h = Math.floor((s % S_PER_DAY) / S_PER_HOUR);
  const m = Math.floor((s % S_PER_HOUR) / S_PER_MIN);
  const sec = s % S_PER_MIN;
  if (d > 0) return `${d}d ${h}h ${m}m`;
  if (h > 0) return `${h}h ${m}m ${sec}s`;
  return `${m}m ${sec}s`;
}

// How long ago the newest successful reading arrived, both in milliseconds
// since the epoch; `lastOkAt` is null before the first one.
export function formatFreshness(lastOkAt, now) {
  if (lastOkAt === null) return "never";
  const age = Math.max(0, Math.round((now - lastOkAt) / MS_PER_S));
  return age < FRESH_AGE_S ? "just now" : `${age}s ago`;
}
