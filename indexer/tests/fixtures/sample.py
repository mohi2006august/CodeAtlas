"""Retry helpers for the billing service."""
import os
from billing.base import BasePolicy, clamp
from . import metrics


def compute_delay(attempt: int, base: float = 0.5) -> float:
    """Exponential backoff for a given attempt."""
    return clamp(base * (2 ** attempt), 0.0, 30.0)


class RetryPolicy(BasePolicy):
    """Decides when a failed payment is retried."""

    def __init__(self, max_attempts: int = 5):
        self.max_attempts = max_attempts

    @property
    def exhausted(self) -> bool:
        return self.attempt >= self.max_attempts

    def next_delay(self, attempt: int) -> float:
        # jitter is applied by the caller
        metrics.record("retry", attempt)
        return compute_delay(attempt)

    async def run(self, job: "Job") -> None:
        def on_error(exc):
            return self.next_delay(1)

        await job.execute(on_error)
