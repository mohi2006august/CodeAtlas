-- | Retry helpers.
module Billing.Retry (RetryPolicy (..), nextDelay) where

import Billing.Math (clamp)
import qualified Data.Map as Map

-- | Decides when a payment is retried.
data RetryPolicy = RetryPolicy
  { maxAttempts :: Int
  }

class Policy p where
  delayFor :: p -> Int -> Int

instance Policy RetryPolicy where
  delayFor _ = nextDelay

-- | Delay before the next attempt.
nextDelay :: Int -> Int
nextDelay 0 = 500
nextDelay attempt = clamp (computeDelay attempt) 0 30000

computeDelay :: Int -> Int
computeDelay attempt = 500 * 2 ^ attempt

defaultPolicy :: RetryPolicy
defaultPolicy = RetryPolicy 5
