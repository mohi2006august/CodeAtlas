//! Retry helpers.
const std = @import("std");
const math = @import("billing/math.zig");

/// Decides when a payment is retried.
pub const RetryPolicy = struct {
    max_attempts: u32,

    /// Delay before the next attempt.
    pub fn nextDelay(self: RetryPolicy, attempt: u32) u64 {
        _ = self;
        return computeDelay(attempt);
    }
};

const Outcome = enum { retry, give_up };

fn computeDelay(attempt: u32) u64 {
    return math.clamp(@as(u64, 500) << attempt, 0, 30000);
}

test "caps delay" {
    try std.testing.expect(computeDelay(20) == 30000);
}
