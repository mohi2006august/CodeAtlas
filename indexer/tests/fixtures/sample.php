<?php
namespace Acme\Billing;

use Acme\Billing\Math\Clamp;
require_once "base_policy.php";

/**
 * Decides when a payment is retried.
 */
class RetryPolicy extends BasePolicy implements Policy
{
    public function __construct(private int $maxAttempts = 5) {}

    // Delay before the next attempt.
    public function nextDelay(int $attempt): int
    {
        Metrics::record("retry", $attempt);
        return $this->computeDelay($attempt);
    }

    private function computeDelay(int $attempt): int
    {
        return clamp(500 << $attempt, 0, 30000);
    }

    public static function create(): self
    {
        return new RetryPolicy(5);
    }
}

interface Policy
{
    public function nextDelay(int $attempt): int;
}

trait Loggable
{
    public function log(string $m): void { error_log($m); }
}

function charge_with_retry($job)
{
    return RetryPolicy::create()->nextDelay($job->attempt);
}
