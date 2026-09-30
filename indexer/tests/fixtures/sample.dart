import 'dart:math';
import 'package:billing/math.dart' as m;

/// Anything that can decide on a retry.
abstract class Policy {
  int nextDelay(int attempt);
}

/// Decides when a payment is retried.
class RetryPolicy extends BasePolicy implements Policy {
  final int maxAttempts;

  RetryPolicy(this.maxAttempts);

  // Delay before the next attempt.
  @override
  int nextDelay(int attempt) {
    Metrics.record('retry', attempt);
    return computeDelay(attempt);
  }

  static RetryPolicy create() => RetryPolicy(5);
}

enum Outcome { retry, giveUp }

mixin Loggable {
  void log(String m) => print(m);
}

int computeDelay(int attempt) {
  return m.clamp(500 << attempt, 0, 30000);
}
