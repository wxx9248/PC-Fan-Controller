<script>
  import { onMount } from "svelte";
  import uPlot from "uplot";
  import "uplot/dist/uPlot.min.css";
  import {
    CHART_HEIGHT_PX,
    CHART_LINE_WIDTH_PX,
    MAX_DUTY_PERCENT,
    RPM_AXIS_HEADROOM,
    RPM_AXIS_MIN_TOP,
  } from "../lib/constants.js";

  let chartEl;
  let chart;

  /** Redraw from `History.columns()`. */
  export function update(columns) {
    chart?.setData(columns);
  }

  onMount(() => {
    // The palette lives in app.css, so the chart and the page cannot drift apart.
    const css = getComputedStyle(document.documentElement);
    const colour = (name) => css.getPropertyValue(name).trim();
    const axisStyle = {
      stroke: colour("--muted"),
      grid: { stroke: colour("--border") },
      ticks: { stroke: colour("--border") },
    };
    chart = new uPlot(
      {
        width: chartEl.clientWidth,
        height: CHART_HEIGHT_PX,
        series: [
          {},
          {
            label: "RPM",
            stroke: colour("--accent"),
            width: CHART_LINE_WIDTH_PX,
            scale: "rpm",
            value: (u, v) => (v == null ? "-" : `${v}`),
          },
          {
            label: "PWM %",
            stroke: colour("--pwm"),
            width: CHART_LINE_WIDTH_PX,
            scale: "pwm",
            value: (u, v) => (v == null ? "-" : `${v}%`),
          },
        ],
        scales: {
          rpm: { range: (u, min, max) => [0, Math.max(max * RPM_AXIS_HEADROOM, RPM_AXIS_MIN_TOP)] },
          pwm: { range: [0, MAX_DUTY_PERCENT] },
        },
        axes: [
          { ...axisStyle },
          { ...axisStyle, scale: "rpm", label: "RPM" },
          { ...axisStyle, scale: "pwm", label: "PWM %", side: 1 },
        ],
        legend: { show: true },
      },
      [[], [], []],
      chartEl
    );

    const resize = () =>
      chart.setSize({ width: chartEl.clientWidth, height: CHART_HEIGHT_PX });
    window.addEventListener("resize", resize);
    return () => {
      window.removeEventListener("resize", resize);
      chart.destroy();
    };
  });
</script>

<section class="panel">
  <div bind:this={chartEl}></div>
</section>

<style>
  .panel {
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1rem;
  }
</style>
