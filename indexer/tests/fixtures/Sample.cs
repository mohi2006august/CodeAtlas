using System;
using Acme.Billing.Math;

namespace Acme.Billing
{
    /// <summary>Decides when a payment is retried.</summary>
    public class RetryPolicy : BasePolicy, IPolicy
    {
        private readonly int _maxAttempts;

        public RetryPolicy(int maxAttempts)
        {
            _maxAttempts = maxAttempts;
        }

        // Delay before the next attempt.
        public override long NextDelay(int attempt)
        {
            Metrics.Record("retry", attempt);
            return ComputeDelay(attempt);
        }

        private static long ComputeDelay(int attempt) => Clamp.Apply(500L << attempt, 0, 30000);

        public static RetryPolicy Create() => new RetryPolicy(5);
    }

    public interface IPolicy
    {
        long NextDelay(int attempt);
    }

    public enum Outcome { Retry, GiveUp }

    public record Attempt(int Number);

    public struct Delay { public long Millis; }
}
