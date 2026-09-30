package com.acme.billing;

import com.acme.billing.math.Clamp;
import java.util.*;

/**
 * Decides when a payment is retried.
 */
public class Sample extends BasePolicy implements Policy {
    private final int maxAttempts;

    public Sample(int maxAttempts) {
        this.maxAttempts = maxAttempts;
    }

    /** Delay before the next attempt. */
    @Override
    public long nextDelay(int attempt) {
        Metrics.record("retry", attempt);
        return computeDelay(attempt);
    }

    static long computeDelay(int attempt) {
        return Clamp.clamp(500L << attempt, 0, 30000);
    }

    interface Listener {
        void onRetry(int attempt);
    }

    enum Outcome { RETRY, GIVE_UP }

    public static Sample create() {
        return new Sample(5);
    }
}
