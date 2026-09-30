(* Retry helpers. *)
open Billing_math

type retry_policy = { max_attempts : int }

(** Backoff for an attempt. *)
let compute_delay attempt = clamp (500 lsl attempt) 0 30000

let next_delay policy attempt =
  Metrics.record "retry" attempt;
  compute_delay attempt

let default_policy = { max_attempts = 5 }

module Registry = struct
  let lookup name = default_policy
end

let rec retry_loop n = if n = 0 then () else retry_loop (n - 1)
