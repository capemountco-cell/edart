import type { PendingBuy } from "../lib/native";
import { formatCents } from "../lib/money";

/**
 * The question the rails ask before a buy, when "Confirm each buy" is on.
 * It sits over the whole window: nothing else matters until it is
 * answered, and no answer within the minute stops the buy.
 */
export function ConfirmBuy({
  pending,
  onAnswer,
}: {
  pending: PendingBuy;
  onAnswer: (approve: boolean) => void;
}) {
  const side =
    pending.buy.side === "up"
      ? "Up"
      : pending.buy.side === "down"
        ? "Down"
        : "Side not read";
  return (
    <div className="modal-layer">
      <div
        className="modal"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="confirm-title"
        aria-describedby="confirm-text"
      >
        <span className="eyebrow">Confirm buy</span>
        <h2 id="confirm-title" className="modal-title">
          {side} · {formatCents(pending.buy.stakeCents)}
        </h2>
        <p className="modal-market">{pending.buy.market}</p>
        <p id="confirm-text" className="fine">
          The agent is about to slide to buy. Edart read the amount on the
          iPhone’s screen and it is within today’s limits. No answer in{" "}
          {pending.secondsLeft} seconds stops the buy.
        </p>
        <div className="modal-actions">
          <button
            type="button"
            className="secondary"
            onClick={() => onAnswer(false)}
          >
            Stop
          </button>
          <button
            type="button"
            className="primary"
            autoFocus
            onClick={() => onAnswer(true)}
          >
            Approve
          </button>
        </div>
      </div>
    </div>
  );
}
