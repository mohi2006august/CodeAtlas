// Retry policy.
#import <Foundation/Foundation.h>
#import "BasePolicy.h"

@protocol Policy
- (NSInteger)nextDelay:(NSInteger)attempt;
@end

@interface RetryPolicy : BasePolicy <Policy>
@property (nonatomic) NSInteger maxAttempts;
- (NSInteger)nextDelay:(NSInteger)attempt;
+ (instancetype)create;
@end

@implementation RetryPolicy

// Delay before the next attempt.
- (NSInteger)nextDelay:(NSInteger)attempt {
    [Metrics record:@"retry" attempt:attempt];
    return computeDelay(attempt);
}

+ (instancetype)create {
    return [[RetryPolicy alloc] init];
}

@end

NSInteger computeDelay(NSInteger attempt) {
    return clamp(500 << attempt, 0, 30000);
}
