import React, { useState } from "react";
import { chargeWithRetry } from "./retry";

interface Props {
  attempt: number;
}

/** Shows the retry countdown. */
export function RetryBanner({ attempt }: Props) {
  const [open, setOpen] = useState(true);
  return open ? <Banner onClose={() => setOpen(false)}>{attempt}</Banner> : null;
}

export const RetryButton: React.FC<Props> = ({ attempt }) => {
  return <button onClick={() => chargeWithRetry({ attempt })}>Retry</button>;
};
