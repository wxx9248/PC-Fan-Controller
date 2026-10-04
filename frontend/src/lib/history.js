// The chart's rolling window of samples, as the columns uPlot draws.
import { HISTORY_SAMPLES, MS_PER_S } from "./constants.js";

export class History {
  #t = [];
  #rpm = [];
  #pwm = [];

  // A null rpm/pwm records "we were here but had no reading", which uPlot draws
  // as a gap. Appending nothing instead lets the chart interpolate straight
  // across an outage, asserting history that was never measured.
  push(rpm, pwm) {
    this.#t.push(Date.now() / MS_PER_S);
    this.#rpm.push(rpm);
    this.#pwm.push(pwm);
    if (this.#t.length > HISTORY_SAMPLES) {
      this.#t.shift();
      this.#rpm.shift();
      this.#pwm.shift();
    }
  }

  columns() {
    return [this.#t, this.#rpm, this.#pwm];
  }
}
