<script>
  let { problem, status } = $props();
</script>

{#if problem}
  <span class="badge bad">{problem}</span>
{:else if status?.failsafe}
  <span class="badge bad">FAILSAFE: no host control</span>
{:else if status}
  <span class="badge good">host control</span>
{/if}

<style>
  .badge {
    padding: 0.2rem 0.6rem;
    border-radius: 999px;
    font-size: 0.8rem;
    font-weight: 600;
  }

  .badge.good {
    background: color-mix(in srgb, var(--good) 18%, transparent);
    color: var(--good);
  }

  .badge.bad {
    background: color-mix(in srgb, var(--bad) 18%, transparent);
    color: var(--bad);
  }
</style>
