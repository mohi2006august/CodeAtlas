import { clamp } from "./math";
import type { Job } from "../jobs/types";

/** Anything that can decide on a retry. */
export interface Policy {
  nextDelay(attempt: number): number;
}

export type Millis = number;

export enum Outcome {
  Retry,
  GiveUp,
}

export abstract class BasePolicy implements Policy {
  abstract nextDelay(attempt: number): number;
}

export class RetryPolicy extends BasePolicy implements Policy {
  constructor(private readonly maxAttempts: number = 5) {
    super();
  }

  // Delay before the next attempt.
  nextDelay(attempt: number): Millis {
    return clamp(500 * 2 ** attempt, 0, 30_000);
  }

  decide = (job: Job): Outcome => {
    return job.attempt < this.maxAttempts ? Outcome.Retry : Outcome.GiveUp;
  };
}

export function chargeWithRetry(job: Job, policy: Policy = new RetryPolicy()): Millis {
  return policy.nextDelay(job.attempt);
}

export namespace Billing {
  export function round(value: number): number {
    return Math.round(value);
  }
}
