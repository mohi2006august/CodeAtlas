import Foundation

/// Anything that can decide on a retry.
protocol Policy {
    func nextDelay(attempt: Int) -> Double
}

/// Decides when a payment is retried.
class RetryPolicy: BasePolicy, Policy {
    let maxAttempts: Int

    init(maxAttempts: Int = 5) {
        self.maxAttempts = maxAttempts
    }

    // Delay before the next attempt.
    func nextDelay(attempt: Int) -> Double {
        Metrics.record("retry", attempt)
        return computeDelay(attempt)
    }
}

struct Outcome {
    var retry: Bool
}

enum Kind {
    case retry, giveUp
}

extension RetryPolicy {
    static func create() -> RetryPolicy {
        return RetryPolicy(maxAttempts: 5)
    }
}

func computeDelay(_ attempt: Int) -> Double {
    return clamp(500 * pow(2.0, Double(attempt)), 0, 30000)
}
