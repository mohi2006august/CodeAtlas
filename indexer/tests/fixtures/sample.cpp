// Retry policy.
#include <chrono>
#include "billing/policy.hpp"

namespace billing {

/// Decides when a payment is retried.
class RetryPolicy : public BasePolicy {
public:
    explicit RetryPolicy(int max_attempts) : max_attempts_(max_attempts) {}

    int next_delay(int attempt) const override;

    bool exhausted(int attempt) const { return attempt >= max_attempts_; }

private:
    int max_attempts_;
};

int RetryPolicy::next_delay(int attempt) const {
    metrics::record("retry", attempt);
    return compute_delay(attempt);
}

template <typename T>
T clamp_delay(T value) {
    return std::min(value, T(30000));
}

struct Outcome {
    bool retry;
};

}  // namespace billing

int compute_delay(int attempt) {
    auto policy = new billing::RetryPolicy(5);
    return billing::clamp_delay(500 << attempt);
}
