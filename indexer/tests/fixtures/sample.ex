defmodule Billing.RetryPolicy do
  @moduledoc """
  Decides when a payment is retried.
  """
  alias Billing.Math
  import Billing.Metrics, only: [record: 2]

  @doc "Delay before the next attempt."
  def next_delay(attempt) when attempt > 10, do: 30_000

  def next_delay(attempt) do
    record("retry", attempt)
    compute_delay(attempt)
  end

  defp compute_delay(attempt) do
    Math.clamp(500 * Integer.pow(2, attempt), 0, 30_000)
  end

  defmacro retry(do: block) do
    quote do: unquote(block)
  end
end
