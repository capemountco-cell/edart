import { useEffect, useState, type FormEvent, type ReactNode } from "react";

import { AlertIcon, CheckIcon } from "../components/icons";
import { Switch } from "../components/Switch";
import {
  centsField,
  formatCents,
  parseCount,
  parseDollars,
} from "../lib/money";
import type { LedgerEvent, TradingChange } from "../lib/native";
import { errorText, type Edart } from "../lib/useEdart";
import type { Route } from "../App";

/**
 * Trading mode: the rules Edart enforces before any gesture reaches the
 * iPhone, today's count, the playbook the agent is given, and the ledger.
 * Every rule here is the value the Rust side checks against.
 */
export function Trading({
  app,
  go,
}: {
  app: Edart;
  go: (route: Route) => void;
}) {
  const trading = app.trading;
  const [error, setError] = useState<string | null>(null);
  const [playbook, setPlaybook] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [starting, setStarting] = useState(false);

  if (!trading) {
    return (
      <div className="page">
        <header className="page-head">
          <span className="eyebrow">Trading</span>
          <h1>
            Trading rails<span className="dim">.</span>
          </h1>
          <p className="lede">Reading the trading settings…</p>
        </header>
      </div>
    );
  }

  /** Save one rule. Returns what went wrong, if anything. */
  const save = async (change: TradingChange): Promise<string | null> => {
    setError(null);
    try {
      await app.changeTrading(change);
      return null;
    } catch (cause) {
      return errorText(cause);
    }
  };

  const toggle = async (change: TradingChange) => setError(await save(change));

  const savePlaybook = async (event: FormEvent) => {
    event.preventDefault();
    if (playbook === null) return;
    const problem = await save({ playbook });
    setError(problem);
    if (!problem) {
      setPlaybook(null);
      setSaved(true);
    }
  };

  const start = async () => {
    setStarting(true);
    setError(null);
    try {
      await app.startTradingRun();
      go("control");
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setStarting(false);
    }
  };

  const { today } = trading;
  const blocker = !trading.enabled
    ? "Turn trading mode on first."
    : app.agent.running
      ? "The agent is already working."
      : (app.agentBlocker?.text ?? null);
  const otherApp = trading.app.trim().toLowerCase() !== "kalshi";

  return (
    <div className="page">
      <header className="page-head">
        <span className="eyebrow">Trading</span>
        <h1>
          Trading rails<span className="dim">.</span>
        </h1>
        <p className="lede">
          Rules this Mac enforces before any tap reaches the iPhone. They cap
          what can be lost in a day. They don’t make the agent a good trader:
          its reading of a market can be wrong, and a buy can lose its whole
          amount.
        </p>
      </header>

      <section className="group" aria-labelledby="mode-title">
        <h2 id="mode-title" className="eyebrow">
          Trading mode
        </h2>
        <div className="card list">
          <div className="list-row wrap">
            <div className="row-text">
              <strong>Trading mode</strong>
              <span>
                When on, every tap, swipe and typed word is checked against what
                is on the iPhone’s screen first: the agent’s, your own clicks on
                the mirror, and connected AI tools’.
              </span>
            </div>
            <Switch
              on={trading.enabled}
              label="Trading mode"
              onChange={(enabled) => void toggle({ enabled })}
            />
          </div>
        </div>
      </section>

      <section className="group" aria-labelledby="today-title">
        <h2 id="today-title" className="eyebrow">
          Today
        </h2>
        <div className="meters">
          <Meter
            label="Buys"
            value={`${today.trades}`}
            of={`of ${trading.maxTradesPerDay}`}
            part={today.trades / trading.maxTradesPerDay}
          />
          <Meter
            label="Spent"
            value={formatCents(today.spentCents)}
            of={`of ${formatCents(trading.maxLossCentsPerDay)}`}
            part={today.spentCents / trading.maxLossCentsPerDay}
          />
        </div>
        <p className="fine">
          Worst case today: {formatCents(trading.worstCaseCents)} lost, if every
          buy allowed expires worthless. {trading.tradesLeft} more{" "}
          {trading.tradesLeft === 1 ? "buy fits" : "buys fit"} today. The count
          starts again at midnight.
        </p>
        {trading.ledgerError && (
          <p className="fine error" role="alert">
            {trading.ledgerError}. Nothing can be bought until the ledger can be
            read.
          </p>
        )}
      </section>

      <section className="group" aria-labelledby="rules-title">
        <h2 id="rules-title" className="eyebrow">
          Rules
        </h2>
        <div className="card list">
          <Rule
            id="rule-app"
            title="Allowed app"
            text="The only app that can be opened and used. On the Home Screen only its icon can be tapped; anywhere else, only Home."
            value={trading.app}
            parse={(text) => (text.trim() === "" ? null : { app: text })}
            invalid="Name the one app the agent may use."
            save={save}
          />
          <Rule
            id="rule-market"
            title="Market"
            text="A buy goes through only when the order names this market. Leave empty to allow any market in the app."
            value={trading.market}
            parse={(text) => ({ market: text })}
            invalid=""
            save={save}
          />
          <Rule
            id="rule-stake"
            title="Amount per buy"
            text="The order sheet must read exactly this before the slide. Anything else, or an amount Edart can’t read for certain, is refused. $0.01 to $5.00."
            prefix="$"
            value={centsField(trading.stakeCents)}
            parse={(text) => {
              const cents = parseDollars(text);
              return cents === null ? null : { stakeCents: cents };
            }}
            invalid="Enter an amount in dollars, like 0.10."
            save={save}
          />
          <Rule
            id="rule-trades"
            title="Most buys per day"
            text="1 to 100."
            value={String(trading.maxTradesPerDay)}
            parse={(text) => {
              const count = parseCount(text);
              return count === null ? null : { maxTradesPerDay: count };
            }}
            invalid="Enter a whole number."
            save={save}
          />
          <Rule
            id="rule-spent"
            title="Most spent per day"
            text={`The amounts of the day’s buys, added up. A buy can’t lose more than its amount, so this caps the day’s loss. $0.01 to $100.00. Worst case today: ${formatCents(trading.worstCaseCents)}.`}
            prefix="$"
            value={centsField(trading.maxLossCentsPerDay)}
            parse={(text) => {
              const cents = parseDollars(text);
              return cents === null ? null : { maxLossCentsPerDay: cents };
            }}
            invalid="Enter an amount in dollars, like 1.00."
            save={save}
          />
          <div className="list-row wrap">
            <div className="row-text">
              <strong>Confirm each buy</strong>
              <span>
                Ask you to approve every buy before the slide. Off, buys within
                the rules above go straight through; the rules hold either way.
              </span>
            </div>
            <Switch
              on={trading.confirmEachBuy}
              label="Confirm each buy"
              onChange={(confirmEachBuy) => void toggle({ confirmEachBuy })}
            />
          </div>
          <Rule
            id="rule-steps"
            title="Steps per run"
            text="How many actions a trading run may take before it stops. 10 to 300."
            value={String(trading.maxSteps)}
            parse={(text) => {
              const count = parseCount(text);
              return count === null ? null : { maxSteps: count };
            }}
            invalid="Enter a whole number."
            save={save}
          />
        </div>
        <p className="fine">
          Changes hold from the next action on, even during a run. Also always
          refused: typing on the order sheet, and deposits, withdrawals and
          transfers.
        </p>
        {otherApp && (
          <p className="notice-line" role="note">
            <AlertIcon size={18} />
            <span>
              Edart knows Kalshi’s screens. In {trading.app}, only screens that
              show its name can be used, and a buy goes through only where Edart
              can read a “Slide to buy” order and its amount. Any other buy is
              blocked.
            </span>
          </p>
        )}
        {error && (
          <p className="fine error" role="alert">
            {error}
          </p>
        )}
      </section>

      <section className="group" aria-labelledby="playbook-title">
        <h2 id="playbook-title" className="eyebrow">
          Playbook
        </h2>
        <form className="card playbook" onSubmit={(e) => void savePlaybook(e)}>
          <label htmlFor="playbook" className="row-text">
            <strong>What the agent is told</strong>
            <span>
              Sent with every step while trading mode is on, with the rules
              above. It guides the agent; the rules are what stop it.
            </span>
          </label>
          <textarea
            id="playbook"
            className="textarea"
            rows={10}
            spellCheck={false}
            value={playbook ?? trading.playbook}
            onChange={(e) => {
              setPlaybook(e.target.value);
              setSaved(false);
            }}
          />
          <div className="row-controls end">
            {saved && playbook === null && (
              <p className="row-note good inline">
                <CheckIcon size={16} />
                Saved
              </p>
            )}
            {!trading.playbookIsDefault && (
              <button
                type="button"
                className="secondary small"
                onClick={() => {
                  setPlaybook(null);
                  void toggle({ playbook: "" });
                }}
              >
                Use built-in
              </button>
            )}
            <button
              type="submit"
              className="secondary small"
              disabled={playbook === null || playbook === trading.playbook}
            >
              Save playbook
            </button>
          </div>
        </form>
        <div className="run-row">
          <button
            type="button"
            className="primary"
            disabled={blocker !== null || starting}
            onClick={() => void start()}
          >
            {starting ? "Starting…" : "Start trading run"}
          </button>
          <p className="fine">
            {blocker ??
              `Starts the agent with the playbook, for up to ${trading.maxSteps} steps. Watch it, and stop it, on the Control screen.`}
          </p>
        </div>
      </section>

      <section className="group" aria-labelledby="ledger-title">
        <h2 id="ledger-title" className="eyebrow">
          Ledger
        </h2>
        {app.ledger.length === 0 ? (
          <p className="fine">
            Nothing yet. Buys, sells and refused actions are listed here, and
            kept in a file only you can read.
          </p>
        ) : (
          <ul className="card ledger">
            {app.ledger.map((event, index) => (
              <LedgerRow key={`${event.time}-${index}`} event={event} />
            ))}
          </ul>
        )}
        <p className="fine">
          A buy is counted when Edart sends the slide, whether or not{" "}
          {trading.app} then fills it, so the count can run ahead of your real
          trades but never behind.
        </p>
      </section>
    </div>
  );
}

function Meter({
  label,
  value,
  of,
  part,
}: {
  label: string;
  value: string;
  of: string;
  part: number;
}) {
  const width = `${Math.round(Math.min(1, Math.max(0, part)) * 100)}%`;
  return (
    <div className="card meter">
      <span className="eyebrow">{label}</span>
      <p className="meter-value">
        {value} <span>{of}</span>
      </p>
      <span className="meter-bar" aria-hidden="true">
        <span style={{ width }} className={part >= 1 ? "full" : ""} />
      </span>
    </div>
  );
}

/**
 * One editable rule: a field that saves when left or on Return, and says
 * whether the value was taken.
 */
function Rule({
  id,
  title,
  text,
  value,
  prefix,
  parse,
  invalid,
  save,
}: {
  id: string;
  title: string;
  text: ReactNode;
  /** The saved value, as the field shows it. */
  value: string;
  prefix?: string;
  /** The change a typed value means, or `null` when it makes no sense. */
  parse: (text: string) => TradingChange | null;
  invalid: string;
  save: (change: TradingChange) => Promise<string | null>;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  const [note, setNote] = useState<{ good: boolean; text: string } | null>(
    null,
  );
  // A complaint about an old value goes when the saved value changes.
  useEffect(() => setNote((n) => (n?.good ? n : null)), [value]);

  const commit = async () => {
    if (draft === null || draft.trim() === value) {
      setDraft(null);
      return;
    }
    const change = parse(draft);
    if (change === null) {
      setNote({ good: false, text: invalid });
      return;
    }
    const problem = await save(change);
    if (problem) {
      setNote({ good: false, text: problem });
      return;
    }
    setDraft(null);
    setNote({ good: true, text: "Saved" });
  };

  return (
    <form
      className="list-row wrap"
      onSubmit={(event) => {
        event.preventDefault();
        void commit();
      }}
    >
      <div className="row-text">
        <label htmlFor={id}>
          <strong>{title}</strong>
        </label>
        <span>{text}</span>
      </div>
      <div className="rule-field">
        {prefix && <span aria-hidden="true">{prefix}</span>}
        <input
          id={id}
          className="field"
          inputMode={prefix ? "decimal" : undefined}
          autoComplete="off"
          spellCheck={false}
          value={draft ?? value}
          onChange={(e) => {
            setDraft(e.target.value);
            setNote(null);
          }}
          onBlur={() => void commit()}
        />
      </div>
      {note && (
        <p
          className={`row-note ${note.good ? "good" : "bad"}`}
          role={note.good ? "status" : "alert"}
        >
          {note.good && <CheckIcon size={16} />}
          {note.text}
        </p>
      )}
    </form>
  );
}

const KIND: Record<LedgerEvent["kind"], string> = {
  buy: "Buy",
  sell: "Sell",
  blocked: "Refused",
};

function LedgerRow({ event }: { event: LedgerEvent }) {
  const when = new Date(event.time * 1000);
  const time = when.toLocaleTimeString([], {
    hour: "numeric",
    minute: "2-digit",
  });
  const side = event.side === "up" ? "Up" : event.side === "down" ? "Down" : "";
  const what =
    event.kind === "blocked"
      ? (event.reason ?? "Refused")
      : [
          side,
          event.kind === "buy" && formatCents(event.stakeCents),
          event.market,
        ]
          .filter(Boolean)
          .join(" · ") || (event.kind === "sell" ? "Sold or cashed out" : "");
  return (
    <li className="ledger-row">
      <span className={`ledger-kind ${event.kind}`}>{KIND[event.kind]}</span>
      <span className="ledger-what">{what}</span>
      <time className="ledger-time" dateTime={when.toISOString()}>
        {event.day.slice(5)} {time}
      </time>
    </li>
  );
}
