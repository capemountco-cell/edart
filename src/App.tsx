import { useCallback, useEffect, useState } from "react";

import {
  bluetoothStatus,
  nativeAvailable,
  perform,
  startBluetooth,
  type BluetoothStatus,
  type Gesture,
} from "./lib/native";
import { phaseCopy } from "./lib/phase";

/** Coordinates for the test buttons, in iPhone points (proportions matter). */
const PHONE = { width: 390, height: 844 };

/** `?demo` shows sample data in a browser tab, for screenshots and design. */
const DEMO: BluetoothStatus = {
  phase: "connected",
  supported: true,
  state: "poweredOn",
  published: true,
  advertising: true,
  subscribed: true,
  failedServices: [],
  error: null,
};

const isDemo = () =>
  typeof window !== "undefined" && new URLSearchParams(window.location.search).has("demo");

const TESTS: { label: string; gesture: Gesture }[] = [
  { label: "Tap the center", gesture: { type: "tap", x: 195, y: 422 } },
  { label: "Go Home", gesture: { type: "press_home" } },
  {
    label: "Swipe up",
    gesture: { type: "swipe", from_x: 195, from_y: 640, to_x: 195, to_y: 240, ms: 300 },
  },
  { label: "Type “hello”", gesture: { type: "type_text", text: "hello" } },
];

export function App() {
  const [status, setStatus] = useState<BluetoothStatus | null>(isDemo() ? DEMO : null);
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<{ kind: "ok" | "error"; text: string } | null>(null);

  const refresh = useCallback(async () => {
    if (isDemo() || !nativeAvailable()) return;
    try {
      setStatus(await bluetoothStatus());
    } catch (cause) {
      setMessage({ kind: "error", text: String(cause) });
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), 1000);
    return () => window.clearInterval(timer);
  }, [refresh]);

  const phase = status?.phase ?? (nativeAvailable() ? "starting" : "unsupported");
  const copy = phaseCopy[phase];
  const connected = phase === "connected";

  const turnOn = async () => {
    setMessage(null);
    try {
      await startBluetooth();
      await refresh();
    } catch (cause) {
      setMessage({ kind: "error", text: cause instanceof Error ? cause.message : String(cause) });
    }
  };

  const run = async (label: string, gesture: Gesture) => {
    setBusy(label);
    setMessage(null);
    try {
      if (!isDemo()) await perform(gesture, PHONE);
      setMessage({ kind: "ok", text: `Sent: ${label}` });
    } catch (cause) {
      setMessage({ kind: "error", text: cause instanceof Error ? cause.message : String(cause) });
    } finally {
      setBusy(null);
    }
  };

  return (
    <main className="page">
      <div className="column">
        <header className="intro">
          <span className="wordmark">EDART</span>
          <span className="eyebrow">Setup · Bluetooth</span>
          <h1>
            Connect your iPhone<span className="dim">.</span>
          </h1>
          <p className="lede">
            Edart becomes a Bluetooth mouse and keyboard for your iPhone. Nothing is installed on
            the phone and nothing leaves this Mac.
          </p>
        </header>

        <section className={`card status status-${phase}`} aria-live="polite">
          <span className="dot" aria-hidden="true" />
          <div className="status-text">
            <h2>{copy.title}</h2>
            <p>{copy.detail}</p>
            {status?.error && phase === "broken" && <p className="fine">{status.error}</p>}
          </div>
          {phase === "notStarted" && (
            <button type="button" className="primary" onClick={() => void turnOn()}>
              Turn on
            </button>
          )}
        </section>

        <section aria-labelledby="steps-title">
          <h2 id="steps-title" className="eyebrow">
            On your iPhone
          </h2>
          <ol className="card steps">
            <li>
              <span className="num">1</span>
              <div>
                <strong>Turn on AssistiveTouch</strong>
                <span>Settings › Accessibility › Touch › AssistiveTouch.</span>
              </div>
            </li>
            <li>
              <span className="num">2</span>
              <div>
                <strong>Pair with Edart</strong>
                <span>In AssistiveTouch, open Devices › Bluetooth Devices and tap “Edart”.</span>
              </div>
            </li>
            <li>
              <span className="num">3</span>
              <div>
                <strong>Try it below</strong>
                <span>A pointer appears on the iPhone once it’s connected.</span>
              </div>
            </li>
          </ol>
        </section>

        <section aria-labelledby="try-title">
          <h2 id="try-title" className="eyebrow">
            Try it
          </h2>
          <div className="card tests">
            {TESTS.map((t) => (
              <button
                key={t.label}
                type="button"
                className="secondary"
                disabled={!connected || busy !== null}
                onClick={() => void run(t.label, t.gesture)}
              >
                {busy === t.label ? "Sending…" : t.label}
              </button>
            ))}
          </div>
          {!connected && <p className="fine">These work once your iPhone is connected.</p>}
          {message && (
            <p className={message.kind === "error" ? "fine error" : "fine ok"} role="status">
              {message.text}
            </p>
          )}
        </section>
      </div>
    </main>
  );
}
