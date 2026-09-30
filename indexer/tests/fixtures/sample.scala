package com.acme.billing

import com.acme.billing.math.Clamp
import scala.concurrent.duration._

/** Anything that can decide on a retry. */
trait Policy {
  def nextDelay(attempt: Int): Long
}

/** Decides when a payment is retried. */
class RetryPolicy(maxAttempts: Int = 5) extends BasePolicy with Policy {
  // Delay before the next attempt.
  override def nextDelay(attempt: Int): Long = {
    Metrics.record("retry", attempt)
    RetryPolicy.computeDelay(attempt)
  }
}

object RetryPolicy {
  def computeDelay(attempt: Int): Long =
    Clamp.clamp(500L << attempt, 0, 30000)

  def create(): RetryPolicy = new RetryPolicy(5)
}

case class Outcome(retry: Boolean)
