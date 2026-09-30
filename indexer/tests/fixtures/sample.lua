-- Retry helpers.
local math2 = require("billing.math")
local metrics = require "metrics"

local RetryPolicy = {}
RetryPolicy.__index = RetryPolicy

-- Backoff for an attempt.
local function compute_delay(attempt)
  return math2.clamp(500 * 2 ^ attempt, 0, 30000)
end

function RetryPolicy.new(max_attempts)
  return setmetatable({ max_attempts = max_attempts }, RetryPolicy)
end

-- Delay before the next attempt.
function RetryPolicy:next_delay(attempt)
  metrics.record("retry", attempt)
  return compute_delay(attempt)
end

local on_error = function(err)
  return RetryPolicy.new(5):next_delay(1)
end

return RetryPolicy
