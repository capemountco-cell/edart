import { useCallback, useEffect, useState } from "react";

import {
  bluetoothStatus,
  nativeAvailable,
  perform,
  openBluetoothSettings,
  reachOut,
  restartBluetooth,
  startBluetooth,
  usbEnableDeveloperMode,
  usbPrepare,
  usbStatus,
  type BluetoothStatus,
  type Gesture,
  type UsbPhase,
  type UsbStatus,
} from "./lib/native";
import { phaseCopy, usbPhaseCopy, usbStepStates } from "./lib/phase";

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
  phoneSeen: true,
  macName: "Nick’s MacBook",
  failedServices: [],
  events: [
    "    0.0s  started",
    "    0.1s  Bluetooth poweredOn",
    "    0.2s  service added 00001812-0000-1000-8000-00805F9B34FB",
    "    0.3s  advertising",
    "   14.2s  subscribe 2A4D by 6F1C2A90",
  ],
  error: null,
};

const DEMO_USB: UsbStatus = {
  phase: "ready",
  deviceName: "Nick’s iPhone",
  iosVersion: "27.3",
  developerMode: true,
  coreDeviceVersion: "701.2",
  error: null,
  events: [
    "    0.0s  watching for an iPhone on USB",
    "    0.4s  iPhone plugged in (00008140)",
    "    0.6s  Nick’s iPhone · iOS 27.3 · Connected · Developer Mode on",
    "    3.1s  connected to the iPhone through Xcode’s device service",
    "    3.4s  developer services ready: touch and keyboard available",
  ],
};

/** Phases in which the phone is on the cable, trusted and unlocked. */
const CAN_CONNECT: UsbPhase[] = [
  "developerModeOff",
  "developerModeManual",
  "confirmDeveloperMode",
  "xcodeMissing",
  "notPrepared",
  "diskImageMissing",
  "serviceUnavailable",
];

const isDemo = () =>
  typeof window !== "undefined" &&
  new URLSearchParams(window.location.search).has("demo");

const TESTS: { label: string; gesture: Gesture }[] = [
  { label: "Tap the center", gesture: { type: "tap", x: 195, y: 422 } },
  { label: "Go Home", gesture: { type: "press_home" } },
  {
    label: "Swipe up",
    gesture: {
      type: "swipe",
      from_x: 195,
      from_y: 640,
      to_x: 195,
      to_y: 240,
      ms: 300,
    },
  },
  { label: "Type “hello”", gesture: { type: "type_text", text: "hello" } },
];

export function App() {
  const [status, setStatus] = useState<BluetoothStatus | null>(
    isDemo() ? DEMO : null,
  );
  const [usb, setUsb] = useState<UsbStatus | null>(isDemo() ? DEMO_USB : null);
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<{
    kind: "ok" | "error";
    text: string;
  } | null>(null);

  const refresh = useCallback(async () => {
    if (isDemo() || !nativeAvailable()) return;
    try {
      const [bluetooth, cable] = await Promise.all([
        bluetoothStatus(),
        usbStatus(),
      ]);
      setStatus(bluetooth);
      setUsb(cable);
    } catch (cause) {
      setMessage({ kind: "error", text: String(cause) });
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), 1000);
    return () => window.clearInterval(timer);
  }, [refresh]);

  const phase =
    status?.phase ?? (nativeAvailable() ? "starting" : "unsupported");
  const mac = status?.macName ?? "your Mac’s name";
  const copy = phaseCopy(phase, mac);
  const connected = phase === "connected";

  const usbPhase: UsbPhase =
    usb?.phase ?? (nativeAvailable() ? "noDevice" : "unsupported");
  const phone = usb?.deviceName ?? "your iPhone";
  const usbCopy = usbPhaseCopy(usbPhase, phone);
  const steps = usbStepStates(usbPhase);
  const usbReady = usbPhase === "ready";
  const canConnect = CAN_CONNECT.includes(usbPhase) && busy === null;
  const canTry = usbReady || connected;

  const fail = (cause: unknown) =>
    setMessage({
      kind: "error",
      text: cause instanceof Error ? cause.message : String(cause),
    });

  const turnOnDeveloperMode = async () => {
    setBusy("developerMode");
    setMessage(null);
    try {
      const outcome = await usbEnableDeveloperMode();
      setMessage({
        kind: "ok",
        text:
          outcome === "restarting"
            ? "Your iPhone is restarting. Unlock it when it’s back."
            : "Developer Mode now shows in the iPhone’s Settings › Privacy & Security. Switch it on there and tap Restart.",
      });
    } catch (cause) {
      fail(cause);
    } finally {
      setBusy(null);
      await refresh();
    }
  };

  const connectUsb = async () => {
    setBusy("connect");
    setMessage(null);
    try {
      const working = usbPrepare();
      // Show "Connecting…" while the worker is busy.
      window.setTimeout(() => void refresh(), 300);
      await working;
      setMessage({ kind: "ok", text: "Connected. Try a tap below." });
    } catch (cause) {
      fail(cause);
    } finally {
      setBusy(null);
      await refresh();
    }
  };

  const turnOn = async () => {
    setMessage(null);
    try {
      await startBluetooth();
      await refresh();
    } catch (cause) {
      setMessage({
        kind: "error",
        text: cause instanceof Error ? cause.message : String(cause),
      });
    }
  };

  const restart = async () => {
    setMessage(null);
    try {
      await restartBluetooth();
      setMessage({
        kind: "ok",
        text: "Bluetooth services restarted. Try pairing again.",
      });
      await refresh();
    } catch (cause) {
      setMessage({
        kind: "error",
        text: cause instanceof Error ? cause.message : String(cause),
      });
    }
  };

  const connectFromMac = async () => {
    setMessage(null);
    try {
      await reachOut();
      setMessage({
        kind: "ok",
        text: "Asked the Mac to connect to your iPhone. Check Details.",
      });
      await refresh();
    } catch (cause) {
      setMessage({
        kind: "error",
        text: cause instanceof Error ? cause.message : String(cause),
      });
    }
  };

  const openSettings = async () => {
    setMessage(null);
    try {
      await openBluetoothSettings();
    } catch (cause) {
      setMessage({
        kind: "error",
        text: cause instanceof Error ? cause.message : String(cause),
      });
    }
  };

  const usbLog = (usb?.events ?? []).join("\n");
  const bluetoothLog = (status?.events ?? []).join("\n");
  const copyLog = async () => {
    const text = [
      `USB (${usbPhase}${usb?.coreDeviceVersion ? `, CoreDevice ${usb.coreDeviceVersion}` : ""})`,
      usbLog,
      "",
      `Bluetooth (${phase})`,
      bluetoothLog,
    ].join("\n");
    try {
      await navigator.clipboard.writeText(text);
      setMessage({ kind: "ok", text: "Details copied." });
    } catch {
      setMessage({
        kind: "error",
        text: "Couldn’t copy. Select the text and copy it instead.",
      });
    }
  };

  const run = async (label: string, gesture: Gesture) => {
    setBusy(label);
    setMessage(null);
    try {
      if (!isDemo()) await perform(gesture, PHONE);
      setMessage({ kind: "ok", text: `Sent: ${label}` });
    } catch (cause) {
      setMessage({
        kind: "error",
        text: cause instanceof Error ? cause.message : String(cause),
      });
    } finally {
      setBusy(null);
    }
  };

  return (
    <main className="page">
      <div className="column">
        <header className="intro">
          <span className="wordmark">EDART</span>
          <span className="eyebrow">Setup · USB</span>
          <h1>
            Connect your iPhone<span className="dim">.</span>
          </h1>
          <p className="lede">
            Edart taps, swipes and types on your iPhone through the cable, the
            way Apple’s developer tools do. Nothing is installed on the phone
            and nothing leaves this Mac.
          </p>
        </header>

        <section
          className={`card status usb-${usbPhase}`}
          aria-live="polite"
          aria-label="USB connection"
        >
          <span className="dot" aria-hidden="true" />
          <div className="status-text">
            <h2>{usbCopy.title}</h2>
            <p>{usbCopy.detail}</p>
            {usb?.error && <p className="fine error">{usb.error}</p>}
          </div>
          {usbPhase === "developerModeOff" && (
            <button
              type="button"
              className="primary"
              disabled={busy !== null}
              onClick={() => void turnOnDeveloperMode()}
            >
              {busy === "developerMode" ? "Asking…" : "Turn on"}
            </button>
          )}
          {(usbPhase === "notPrepared" ||
            usbPhase === "diskImageMissing" ||
            usbPhase === "serviceUnavailable" ||
            usbPhase === "xcodeMissing") && (
            <button
              type="button"
              className="primary"
              disabled={!canConnect}
              onClick={() => void connectUsb()}
            >
              {busy === "connect" ? "Connecting…" : "Connect"}
            </button>
          )}
        </section>

        <section aria-labelledby="steps-title">
          <h2 id="steps-title" className="eyebrow">
            Set up in four steps
          </h2>
          <ol className="card steps">
            <li className={steps[0]}>
              <span className="num">{steps[0] === "done" ? "✓" : "1"}</span>
              <div>
                <strong>Plug in your iPhone and tap Trust</strong>
                <span>
                  Use a USB cable. Unlock the iPhone, tap Trust and enter its
                  passcode.
                </span>
              </div>
            </li>
            <li className={steps[1]}>
              <span className="num">{steps[1] === "done" ? "✓" : "2"}</span>
              <div>
                <strong>Turn on Developer Mode</strong>
                <span>
                  The iPhone only accepts touches from a Mac in Developer Mode.
                  Edart asks the iPhone to switch it on. If the iPhone has a
                  passcode, Edart makes the switch appear in Settings › Privacy
                  &amp; Security › Developer Mode for you to flip.
                </span>
                {steps[1] === "current" && (
                  <button
                    type="button"
                    className="secondary step-button"
                    disabled={busy !== null}
                    onClick={() => void turnOnDeveloperMode()}
                  >
                    {busy === "developerMode"
                      ? "Asking…"
                      : "Turn on Developer Mode"}
                  </button>
                )}
              </div>
            </li>
            <li className={steps[2]}>
              <span className="num">{steps[2] === "done" ? "✓" : "3"}</span>
              <div>
                <strong>Restart, then confirm on the iPhone</strong>
                <span>
                  The iPhone restarts to finish. When it’s back, unlock it, tap
                  Turn On and enter the passcode.
                </span>
                {usbPhase === "developerModeManual" && (
                  <button
                    type="button"
                    className="secondary step-button"
                    disabled={busy !== null}
                    onClick={() => void turnOnDeveloperMode()}
                  >
                    Show the switch again
                  </button>
                )}
              </div>
            </li>
            <li className={steps[3]}>
              <span className="num">{steps[3] === "done" ? "✓" : "4"}</span>
              <div>
                <strong>Connect</strong>
                <span>
                  Edart reaches the iPhone’s touch and keyboard through Xcode’s
                  device service, so Xcode must be installed (free on the App
                  Store) and opened once. Connect again after the iPhone
                  restarts.
                </span>
                {!usbReady && (
                  <button
                    type="button"
                    className="secondary step-button"
                    disabled={!canConnect}
                    onClick={() => void connectUsb()}
                  >
                    {busy === "connect" || usbPhase === "preparing"
                      ? "Connecting…"
                      : "Connect"}
                  </button>
                )}
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
                disabled={!canTry || busy !== null}
                onClick={() => void run(t.label, t.gesture)}
              >
                {busy === t.label ? "Sending…" : t.label}
              </button>
            ))}
          </div>
          {!canTry && (
            <p className="fine">These work once your iPhone is connected.</p>
          )}
          {usbReady && (
            <p className="fine">
              Keep the iPhone unlocked and upright. Typing needs a text field
              with the keyboard showing.
            </p>
          )}
          {message && (
            <p
              className={message.kind === "error" ? "fine error" : "fine ok"}
              role="status"
            >
              {message.text}
            </p>
          )}
        </section>

        <details className="card details">
          <summary>Bluetooth (alternative)</summary>
          <p className="fine intro-note">
            Without a cable, Edart can act as a Bluetooth mouse and keyboard
            instead. It does not work when the iPhone and this Mac are signed in
            to the same Apple Account.
          </p>
          <div className={`status inner status-${phase}`} aria-live="polite">
            <span className="dot" aria-hidden="true" />
            <div className="status-text">
              <h3>{copy.title}</h3>
              <p>{copy.detail}</p>
              {status?.error && phase === "broken" && (
                <p className="fine">{status.error}</p>
              )}
            </div>
            {phase === "notStarted" && (
              <button
                type="button"
                className="secondary"
                onClick={() => void turnOn()}
              >
                Turn on
              </button>
            )}
            {(phase === "waitingForPhone" ||
              phase === "pairing" ||
              phase === "broken") && (
              <button
                type="button"
                className="secondary"
                onClick={() => void restart()}
              >
                Restart Bluetooth
              </button>
            )}
            {(phase === "waitingForPhone" || phase === "pairing") && (
              <button
                type="button"
                className="secondary"
                onClick={() => void connectFromMac()}
              >
                Connect from Mac
              </button>
            )}
          </div>
          <ol className="steps inner">
            <li>
              <span className="num">1</span>
              <div>
                <strong>On this Mac, open Bluetooth settings</strong>
                <span>
                  Keep the window open while pairing. Without it, macOS quietly
                  refuses the iPhone’s pairing request.
                </span>
              </div>
              <button
                type="button"
                className="secondary step-action"
                onClick={() => void openSettings()}
              >
                Open
              </button>
            </li>
            <li>
              <span className="num">2</span>
              <div>
                <strong>On the iPhone, turn on AssistiveTouch</strong>
                <span>Settings › Accessibility › Touch › AssistiveTouch.</span>
              </div>
            </li>
            <li>
              <span className="num">3</span>
              <div>
                <strong>Pair with “{mac}”</strong>
                <span>
                  In AssistiveTouch, open Devices › Bluetooth Devices and tap
                  it, then accept Pair on both devices. If it already says
                  Connected, tap ⓘ › Forget This Device first. The iPhone shows
                  the Mac’s name, not “Edart”, and pairing from Settings ›
                  Bluetooth won’t work.
                </span>
              </div>
            </li>
            <li>
              <span className="num">4</span>
              <div>
                <strong>Try it above</strong>
                <span>
                  A pointer appears on the iPhone once it’s connected.
                </span>
              </div>
            </li>
          </ol>
        </details>

        {(usbLog || bluetoothLog) && (
          <details className="card details">
            <summary>Details</summary>
            {usbLog && (
              <>
                <h3 className="log-title">USB</h3>
                <pre className="log">{usbLog}</pre>
              </>
            )}
            {bluetoothLog && (
              <>
                <h3 className="log-title">Bluetooth</h3>
                <pre className="log">{bluetoothLog}</pre>
              </>
            )}
            <button
              type="button"
              className="secondary"
              onClick={() => void copyLog()}
            >
              Copy details
            </button>
          </details>
        )}
      </div>
    </main>
  );
}
