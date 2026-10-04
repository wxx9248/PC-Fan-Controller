<script>
  import { formatUptime } from "../lib/format.js";

  let { status } = $props();
</script>

<section class="cards">
  <div class="card">
    <span class="label">Fan speed</span>
    <span class="value">{status.rpm}<small> RPM</small></span>
  </div>
  <div class="card">
    <span class="label">PWM duty</span>
    <span class="value">{status.pwmPercent}<small> %</small></span>
  </div>
  <div class="card">
    <span class="label">Uptime</span>
    <span class="value small">{formatUptime(status.uptimeSeconds)}</span>
  </div>
  <div class="card">
    <span class="label">WiFi signal</span>
    <span class="value small">
      {status.wifiRssiDbm == null ? "—" : `${status.wifiRssiDbm} dBm`}
    </span>
  </div>
</section>

<style>
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
    gap: 0.8rem;
    margin: 1.2rem 0;
  }

  .card {
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 0.9rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }

  .label {
    color: var(--muted);
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .value {
    font-size: 1.8rem;
    font-weight: 700;
    color: var(--accent);
  }

  .value.small {
    font-size: 1.2rem;
    color: var(--text);
  }

  .value small {
    font-size: 0.9rem;
    font-weight: 400;
    color: var(--muted);
  }
</style>
