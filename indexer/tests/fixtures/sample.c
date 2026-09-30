/* Retry helpers. */
#include <stdio.h>
#include "billing/math.h"

#define MAX_DELAY(x) ((x) > 30000 ? 30000 : (x))

typedef struct retry_policy {
    int max_attempts;
} retry_policy_t;

enum outcome { RETRY, GIVE_UP };

/* Backoff for an attempt. */
static int compute_delay(int attempt) {
    return clamp(500 << attempt, 0, 30000);
}

int *next_delay(const retry_policy_t *policy, int attempt) {
    record_metric("retry", attempt);
    printf("%d\n", compute_delay(attempt));
    return NULL;
}
