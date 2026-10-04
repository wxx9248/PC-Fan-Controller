// Every fixed value the status page depends on, in one place. Plain values
// only, with no Vite-specific code, so the mock and its tests under Node can
// import it too. Colours are not here: they are CSS custom properties in
// app.css, which the chart reads at mount.

// ---- The controller's HTTP API (PROTOCOL.md, "HTTP API") ----

// Same-origin on the device. A build can point elsewhere with VITE_API_BASE.
export const DEFAULT_API_BASE = "/api";
export const STATUS_RESOURCE = "/status";
export const STATUS_PATH = DEFAULT_API_BASE + STATUS_RESOURCE;

// The duty range PROTOCOL.md specifies for pwmPercent.
export const MAX_DUTY_PERCENT = 100;

// ---- Time ----

export const MS_PER_S = 1000;
export const S_PER_MIN = 60;
export const S_PER_HOUR = 60 * S_PER_MIN;
export const S_PER_DAY = 24 * S_PER_HOUR;

// ---- Polling ----

export const POLL_INTERVAL_MS = 2000;
// A refused connection rejects promptly; a black-holed one does not, so each
// request needs its own deadline or the page can hang indefinitely on stale data.
export const REQUEST_TIMEOUT_MS = 2 * POLL_INTERVAL_MS;
// How often the "Updated ... ago" line is refreshed.
export const CLOCK_TICK_MS = 1000;
// A reading younger than this is shown as "just now".
export const FRESH_AGE_S = 2;

// ---- Chart ----

export const HISTORY_WINDOW_S = 10 * S_PER_MIN;
export const HISTORY_SAMPLES = (HISTORY_WINDOW_S * MS_PER_S) / POLL_INTERVAL_MS;
export const CHART_HEIGHT_PX = 240;
export const CHART_LINE_WIDTH_PX = 2;
// Room above the highest RPM on screen, and the lowest top the axis may have,
// so an idle or empty chart still has a sensible scale.
export const RPM_AXIS_HEADROOM = 1.1;
export const RPM_AXIS_MIN_TOP = 100;
