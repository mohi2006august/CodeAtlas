# Retry helpers.
library(jsonlite)
source("billing/math.R")

#' Backoff for an attempt.
compute_delay <- function(attempt, base = 500) {
  clamp(base * 2^attempt, 0, 30000)
}

next_delay = function(policy, attempt) {
  metrics::record("retry", attempt)
  compute_delay(attempt)
}
