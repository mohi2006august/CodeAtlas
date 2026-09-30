# Retry helpers.
module Billing

using Dates
import ..Math: clamp
include("metrics.jl")

abstract type AbstractPolicy end

"""
Decides when a payment is retried.
"""
struct RetryPolicy <: AbstractPolicy
    max_attempts::Int
end

# Delay before the next attempt.
function next_delay(policy::RetryPolicy, attempt::Int)
    Metrics.record("retry", attempt)
    return compute_delay(attempt)
end

compute_delay(attempt) = clamp(500 * 2^attempt, 0, 30000)

macro retry(ex)
    return ex
end

end
