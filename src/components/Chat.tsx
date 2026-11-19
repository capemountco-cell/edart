import { useEffect, useRef, useState, type KeyboardEvent } from "react";

import type { AgentStatus } from "../lib/native";
import { errorText, type AgentBlocker } from "../lib/useEdart";
import { AlertIcon, CheckIcon, PlusIcon, SendIcon, StopIcon } from "./icons";

const SUGGESTIONS = [
  "Open Settings and turn on Dark Mode",
  "Read my three newest messages",
  "Test the sign-up flow in my app",
];

/**
 * The agent: what you asked, what it thought and did, and how it ended.
 * The loop itself runs in Rust; this shows its status and sends goals.
 */
export function Chat({
  agent,
  blocker,
  onAsk,
  onStop,
  onNewChat,
  onFix,
  sheet = false,
}: {
  agent: AgentStatus;
  blocker: AgentBlocker | null;
  onAsk: (goal: string) => Promise<void>;
  onStop: () => Promise<void>;
  onNewChat: () => Promise<void>;
  onFix: (route: AgentBlocker["fix"]["route"]) => void;
  /** Shown in the bottom sheet of a narrow window. */
  sheet?: boolean;
}) {
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const log = useRef<HTMLDivElement>(null);
  const field = useRef<HTMLTextAreaElement>(null);
  const empty = agent.entries.length === 0;

  // Keep the newest line in view.
  useEffect(() => {
    const el = log.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [agent.entries.length, agent.running]);

  const canSend =
    !agent.running && !sending && blocker === null && draft.trim() !== "";

  const send = async () => {
    if (!canSend) return;
    setSending(true);
    setError(null);
    try {
      await onAsk(draft.trim());
      setDraft("");
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setSending(false);
    }
  };

  const guard = (work: () => Promise<void>) => async () => {
    setError(null);
    try {
      await work();
    } catch (cause) {
      setError(errorText(cause));
    }
  };

  const onKey = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      void send();
    }
  };

  return (
    <div className={`chat ${sheet ? "in-sheet" : ""}`}>
      <div className="chat-head">
        <span className="chat-title">
          Agent
          {agent.running && (
            <span className="chat-progress">
              Step {Math.max(1, agent.step)} of {agent.maxSteps}
            </span>
          )}
        </span>
        <button
          type="button"
          className="icon-button"
          aria-label="New chat"
          disabled={agent.running || empty}
          onClick={() => void guard(onNewChat)()}
        >
          <PlusIcon />
        </button>
      </div>

      <div className="chat-body" ref={log}>
        {empty ? (
          <div className="chat-empty">
            <span className="eyebrow">Try</span>
            <h2>
              Tell it what to do<span className="dim">.</span>
            </h2>
            {SUGGESTIONS.map((text) => (
              <button
                key={text}
                type="button"
                className="chip"
                onClick={() => {
                  setDraft(text);
                  field.current?.focus();
                }}
              >
                {text}
              </button>
            ))}
          </div>
        ) : (
          <ol className="chat-log" aria-live="polite">
            {agent.entries.map((entry, i) => {
              switch (entry.kind) {
                case "goal":
                  return (
                    <li key={i} className="say goal">
                      {entry.text}
                    </li>
                  );
                case "step":
                  return (
                    <li key={i} className="say step">
                      <span className="step-n">{entry.n}</span>
                      <span className="step-text">
                        <strong>{entry.action}</strong>
                        {entry.thought && <span>{entry.thought}</span>}
                      </span>
                    </li>
                  );
                case "problem":
                  return (
                    <li key={i} className="say problem">
                      <AlertIcon size={18} />
                      <span>{entry.text}</span>
                    </li>
                  );
                case "finished":
                  return (
                    <li
                      key={i}
                      className={`say finished ${entry.success ? "good" : "plain"}`}
                    >
                      {entry.success && <CheckIcon size={18} />}
                      <span>{entry.text}</span>
                    </li>
                  );
              }
            })}
            {agent.running && (
              <li className="say thinking">
                <span className="pulse-dot" aria-hidden="true" />
                {agent.stopping ? "Stopping…" : "Looking at the screen…"}
              </li>
            )}
          </ol>
        )}
      </div>

      <div className="chat-foot">
        {blocker && !agent.running && (
          <div className="notice">
            <span>{blocker.text}</span>
            <button
              type="button"
              className="secondary small"
              onClick={() => onFix(blocker.fix.route)}
            >
              {blocker.fix.label}
            </button>
          </div>
        )}
        {error && (
          <p className="fine error" role="alert">
            {error}
          </p>
        )}
        <div className="composer">
          <label htmlFor={sheet ? "ask-sheet" : "ask"} className="sr-only">
            Message the agent
          </label>
          <textarea
            id={sheet ? "ask-sheet" : "ask"}
            ref={field}
            rows={sheet ? 1 : 2}
            placeholder="Do anything…"
            value={draft}
            disabled={agent.running}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={onKey}
          />
          {agent.running ? (
            <button
              type="button"
              className="send stop"
              aria-label="Stop the agent"
              disabled={agent.stopping}
              onClick={() => void guard(onStop)()}
            >
              <StopIcon />
            </button>
          ) : (
            <button
              type="button"
              className="send"
              aria-label="Send"
              disabled={!canSend}
              onClick={() => void send()}
            >
              <SendIcon />
            </button>
          )}
        </div>
        <p className="fine chat-note">
          Each step sends a screenshot of the iPhone to the AI service.
        </p>
      </div>
    </div>
  );
}
