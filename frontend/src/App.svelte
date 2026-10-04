<script>
  import { onMount } from "svelte";
  import HistoryChart from "./components/HistoryChart.svelte";
  import StatusBadge from "./components/StatusBadge.svelte";
  import StatusCards from "./components/StatusCards.svelte";
  import { fetchStatus } from "./lib/api.js";
  import { CLOCK_TICK_MS, POLL_INTERVAL_MS } from "./lib/constants.js";
  import { formatFreshness } from "./lib/format.js";
  import { History } from "./lib/history.js";
  import { StatusContractError } from "./lib/status.js";

  let status = $state(null);
  // Why the last poll produced nothing usable, or null if it did. A reply that
  // breaks the contract is as useless as no reply, but it points at the firmware
  // rather than the network, so the badge says which.
  let problem = $state(null);
  // Age of the newest successful reading. Without showing this, a connection
  // that is black-holed rather than refused leaves the last values on screen
  // under a healthy badge, with nothing to reveal that they are stale.
  let lastOkAt = $state(null);
  let now = $state(Date.now());
  // Guards against overlapping requests: setInterval fires regardless of whether
  // the previous fetch finished, so a slow link would otherwise keep opening new
  // connections every POLL_INTERVAL_MS against a server that takes at most 7 clients.
  let inFlight = false;

  let chart;
  const history = new History();

  function record(rpm, pwm) {
    history.push(rpm, pwm);
    chart?.update(history.columns());
  }

  async function poll() {
    if (inFlight) return;
    inFlight = true;
    try {
      status = await fetchStatus();
      problem = null;
      lastOkAt = Date.now();
      record(status.rpm, status.pwmPercent);
    } catch (e) {
      if (e instanceof StatusContractError) {
        console.warn(`/api/status: ${e.message}`);
        problem = "invalid response";
      } else {
        problem = "unreachable";
      }
      record(null, null);
    } finally {
      inFlight = false;
    }
  }

  onMount(() => {
    poll();
    const timer = setInterval(poll, POLL_INTERVAL_MS);
    const clock = setInterval(() => (now = Date.now()), CLOCK_TICK_MS);
    return () => {
      clearInterval(timer);
      clearInterval(clock);
    };
  });
</script>

<main>
  <header>
    <h1>🌀 PC Fan Controller</h1>
    <StatusBadge {problem} {status} />
  </header>

  {#if status}
    <StatusCards {status} />
  {:else if !problem}
    <p class="muted">Loading…</p>
  {/if}

  <HistoryChart bind:this={chart} />

  <footer class="muted">
    Read-only view. Fan speed is controlled by the NAS over USB.
    <span class:stale={problem}>Updated {formatFreshness(lastOkAt, now)}.</span>
  </footer>
</main>

<style>
  main {
    max-width: 900px;
    margin: 0 auto;
    padding: 1.5rem 1rem 2rem;
  }

  header {
    display: flex;
    align-items: center;
    gap: 1rem;
    flex-wrap: wrap;
  }

  h1 {
    font-size: 1.3rem;
    margin: 0;
  }

  .muted {
    color: var(--muted);
    font-size: 0.85rem;
  }

  footer {
    margin-top: 1rem;
  }

  .stale {
    color: var(--bad);
    font-weight: 600;
  }
</style>
