#!/usr/bin/env bash
# Retry helpers.
source ./lib/log.sh
. ./lib/math.sh

# Backoff for an attempt.
compute_delay() {
  local attempt=$1
  clamp $((500 << attempt)) 0 30000
}

function charge_with_retry {
  log_info "retrying"
  sleep "$(compute_delay "$1")"
}

charge_with_retry 3
