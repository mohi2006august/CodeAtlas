# Retry helpers for billing.
require "json"
require_relative "base_policy"

module Billing
  # Decides when a payment is retried.
  class RetryPolicy < BasePolicy
    def initialize(max_attempts = 5)
      @max_attempts = max_attempts
    end

    # Delay before the next attempt.
    def next_delay(attempt)
      Metrics.record("retry", attempt)
      compute_delay(attempt)
    end

    def self.create
      new(5)
    end

    private

    def compute_delay(attempt)
      clamp(500 * 2**attempt, 0, 30_000)
    end
  end
end

def charge_with_retry(job)
  Billing::RetryPolicy.create.next_delay(job.attempt)
end
