// Package billing handles payment retries.
package billing

import (
	"time"

	m "github.com/acme/app/metrics"
)

// RetryPolicy decides when a payment is retried.
type RetryPolicy struct {
	MaxAttempts int
}

type Policy interface {
	NextDelay(attempt int) time.Duration
}

// NewRetryPolicy builds a policy.
func NewRetryPolicy(max int) *RetryPolicy {
	return &RetryPolicy{MaxAttempts: max}
}

// NextDelay returns the backoff for an attempt.
func (p *RetryPolicy) NextDelay(attempt int) time.Duration {
	m.Record("retry", attempt)
	return time.Duration(computeDelay(attempt)) * time.Millisecond
}

func computeDelay(attempt int) int {
	return clamp(500<<attempt, 0, 30000)
}
