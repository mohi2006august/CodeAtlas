// Payment retry helpers.
import { clamp } from "./math.js";
const metrics = require("../metrics");

/**
 * Exponential backoff for an attempt.
 */
export function computeDelay(attempt, base = 0.5) {
  return clamp(base * 2 ** attempt, 0, 30);
}

export const withJitter = (delay) => delay * Math.random();

export class RetryPolicy extends BasePolicy {
  constructor(maxAttempts = 5) {
    super();
    this.maxAttempts = maxAttempts;
  }

  /** Delay before the next attempt. */
  nextDelay(attempt) {
    metrics.record("retry", attempt);
    return withJitter(computeDelay(attempt));
  }

  static create() {
    return new RetryPolicy();
  }
}

module.exports.legacyRetry = function (job) {
  return RetryPolicy.create().nextDelay(job.attempt);
};
