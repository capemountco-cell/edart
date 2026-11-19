/**
 * The Connectors card: how AI tools reach the iPhone through Edart.
 *
 * Shows whether the local MCP server is listening, lets the user turn it
 * off, and hands each AI tool the exact text it needs. Mount it with the
 * status the app polls:
 *
 *   <Connectors status={mcp} onChange={setMcp} />
 */

import { useEffect, useState } from "react";

import {
  mcpCallTime,
  mcpHeadline,
  mcpSetEnabled,
  mcpSetups,
  mcpUrl,
  type McpSetup,
  type McpStatus,
} from "../lib/mcp";
import "./connectors.css";

export interface ConnectorsProps {
  /** From `mcpStatus()`; `null` until the first answer. */
  status: McpStatus | null;
  /** Called with the new status right after the switch is used. */
  onChange?: (status: McpStatus) => void;
}

/** Calls shown, newest first. */
const CALLS_SHOWN = 8;

async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    // Older WebKit, or a webview that withholds the clipboard API.
    const area = document.createElement("textarea");
    area.value = text;
    area.setAttribute("readonly", "");
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.appendChild(area);
    area.select();
    let copied = false;
    try {
      copied = document.execCommand("copy");
    } catch {
      copied = false;
    }
    area.remove();
    return copied;
  }
}

function CopyButton({
  text,
  label,
  primary,
}: {
  text: string;
  label: string;
  primary?: boolean;
}) {
  const [state, setState] = useState<"idle" | "copied" | "failed">("idle");

  useEffect(() => {
    if (state === "idle") return;
    const timer = window.setTimeout(() => setState("idle"), 1600);
    return () => window.clearTimeout(timer);
  }, [state]);

  return (
    <button
      type="button"
      className={primary ? "mcp-copy mcp-copy-primary" : "mcp-copy"}
      aria-label={label}
      onClick={async () =>
        setState((await copyText(text)) ? "copied" : "failed")
      }
    >
      <svg
        aria-hidden="true"
        width="18"
        height="18"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        {state === "copied" ? (
          <path d="M5 12.5l4.5 4.5L19 7.5" />
        ) : (
          <>
            <rect x="8" y="8" width="12" height="12" rx="2" />
            <path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2" />
          </>
        )}
      </svg>
      <span aria-live="polite">
        {state === "copied"
          ? "Copied"
          : state === "failed"
            ? "Select and copy"
            : "Copy"}
      </span>
    </button>
  );
}

export function Connectors({ status, onChange }: ConnectorsProps) {
  const [picked, setPicked] = useState<McpSetup["id"]>("claude-code");
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);

  const headline = mcpHeadline(status);
  const url = mcpUrl(status);
  const setups = mcpSetups(status);
  const setup = setups.find((s) => s.id === picked) ?? setups[0]!;
  const calls = status ? status.calls.slice(-CALLS_SHOWN).reverse() : [];
  const on = status?.enabled ?? false;

  const toggle = async () => {
    if (!status || busy) return;
    setBusy(true);
    setFailure(null);
    try {
      onChange?.(await mcpSetEnabled(!status.enabled));
    } catch (e) {
      setFailure(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="mcp" aria-labelledby="mcp-title">
      <h2 id="mcp-title" className="mcp-eyebrow">
        Connectors
      </h2>
      <div className="mcp-card">
        <div className="mcp-intro">
          <h3>Use your iPhone from any AI tool</h3>
          <p>
            Add this MCP server in Claude, Claude Code, Cursor or any MCP
            client. It only answers on this Mac.
          </p>
        </div>

        <div className="mcp-state">
          <span className={`mcp-dot mcp-dot-${headline.tone}`} aria-hidden />
          <p className="mcp-state-text" role="status">
            {headline.text}
          </p>
          <button
            type="button"
            role="switch"
            aria-checked={on}
            aria-label="MCP server"
            className="mcp-switch"
            disabled={!status || busy}
            onClick={toggle}
          >
            <span className="mcp-switch-track" aria-hidden>
              <span className="mcp-switch-thumb" />
            </span>
            <span className="mcp-switch-label">{on ? "On" : "Off"}</span>
          </button>
        </div>
        {failure && <p className="mcp-fine mcp-error">{failure}</p>}

        <div className="mcp-address">
          <code>{url}</code>
          <CopyButton text={url} label="Copy the server address" primary />
        </div>

        <div className="mcp-setup">
          <div className="mcp-tabs" role="tablist" aria-label="Set up in">
            {setups.map((s) => (
              <button
                key={s.id}
                type="button"
                role="tab"
                id={`mcp-tab-${s.id}`}
                aria-selected={s.id === setup.id}
                aria-controls="mcp-panel"
                className="mcp-tab"
                onClick={() => setPicked(s.id)}
              >
                {s.label}
              </button>
            ))}
          </div>
          <div
            id="mcp-panel"
            role="tabpanel"
            aria-labelledby={`mcp-tab-${setup.id}`}
            className="mcp-panel"
          >
            <p className="mcp-fine">{setup.how}</p>
            <pre className="mcp-snippet">{setup.text}</pre>
            <CopyButton
              text={setup.text}
              label={`Copy the setup for ${setup.label}`}
            />
          </div>
        </div>

        {status && status.tools.length > 0 && (
          <div>
            <h4 className="mcp-eyebrow">What AI tools can do</h4>
            <ul className="mcp-tools">
              {status.tools.map((tool) => (
                <li key={tool.name} title={tool.description}>
                  {tool.name}
                </li>
              ))}
            </ul>
          </div>
        )}

        <div>
          <h4 className="mcp-eyebrow">Recent calls</h4>
          {calls.length === 0 ? (
            <p className="mcp-fine">
              Nothing yet. Calls from your AI tools show up here.
            </p>
          ) : (
            <ul className="mcp-calls">
              {calls.map((call, i) => (
                <li key={`${call.atMs}-${i}`}>
                  <time>{mcpCallTime(call.atMs)}</time>
                  <span className="mcp-call-tool">{call.tool}</span>
                  <span
                    className={call.ok ? "mcp-call-ok" : "mcp-call-bad"}
                    title={call.error ?? undefined}
                  >
                    {call.ok ? "ok" : (call.error ?? "failed")}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </section>
  );
}
