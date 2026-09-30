package com.acme.billing

import com.acme.billing.math.clamp
import kotlin.math.pow

/** Anything that can decide on a retry. */
interface Policy {
    fun nextDelay(attempt: Int): Long
}

/**
 * Decides when a payment is retried.
 */
class RetryPolicy(private val maxAttempts: Int = 5) : BasePolicy(), Policy {
    // Delay before the next attempt.
    override fun nextDelay(attempt: Int): Long {
        Metrics.record("retry", attempt)
        return computeDelay(attempt)
    }

    companion object {
        fun create(): RetryPolicy = RetryPolicy(5)
    }
}

object Registry {
    fun lookup(name: String): Policy = RetryPolicy.create()
}

data class Outcome(val retry: Boolean)

fun computeDelay(attempt: Int): Long {
    return clamp(500L shl attempt, 0, 30000)
}
