import { useState } from "react";

import type { Edart } from "../lib/useEdart";

/** The event logs of the three links, for working out what went wrong. */
export function Details({ app, open = false }: { app: Edart; open?: boolean }) {
  const [copied, setCopied] = useState<"ok" | "failed" | null>(null);
  const logs = [
    {
      title: "Screen",
      state: app.mirror?.phase ?? "unknown",
      lines: app.mirror?.events ?? [],
    },
    {
      title: "Bluetooth",
      state: app.bluetooth?.phase ?? "unknown",
      lines: app.bluetooth?.events ?? [],
    },
    {
      title: "USB control",
      state: `${app.usb?.phase ?? "unknown"}${
        app.usb?.coreDeviceVersion
          ? `, CoreDevice ${app.usb.coreDeviceVersion}`
          : ""
      }`,
      lines: app.usb?.events ?? [],
    },
  ].filter((log) => log.lines.length > 0);
  if (logs.length === 0) return null;

  const copy = async () => {
    const text = logs
      .map((log) => [`${log.title} (${log.state})`, ...log.lines].join("\n"))
      .join("\n\n");
    try {
      await navigator.clipboard.writeText(text);
      setCopied("ok");
    } catch {
      setCopied("failed");
    }
  };

  return (
    <details className="card details" open={open}>
      <summary>Details</summary>
      {logs.map((log) => (
        <div key={log.title}>
          <h3 className="log-title">{log.title}</h3>
          <pre className="log">{log.lines.join("\n")}</pre>
        </div>
      ))}
      <div className="details-foot">
        <button
          type="button"
          className="secondary small"
          onClick={() => void copy()}
        >
          Copy details
        </button>
        {copied && (
          <span
            className={copied === "ok" ? "fine ok" : "fine error"}
            role="status"
          >
            {copied === "ok"
              ? "Copied."
              : "Couldn’t copy. Select the text and copy it instead."}
          </span>
        )}
      </div>
    </details>
  );
}
