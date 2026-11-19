import { useState, type ReactNode } from "react";

import { BluetoothIcon, CameraIcon, CheckIcon } from "../components/icons";
import { Details } from "../components/Details";
import {
  openBluetoothSettings,
  openCameraSettings,
  reachOut,
  restartBluetooth,
  startBluetooth,
  startMirror,
  type Gesture,
} from "../lib/native";
import { mirrorCopy, phaseCopy, type StepState } from "../lib/phase";
import { edgeSwipe } from "../lib/pointer";
import { errorText, type Edart } from "../lib/useEdart";
import type { Route } from "../App";

/**
 * Getting an iPhone connected: the screen over the cable, touch over
 * Bluetooth, and the permissions each needs.
 */
export function Setup({ app, go }: { app: Edart; go: (route: Route) => void }) {
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<{
    kind: "ok" | "error";
    text: string;
  } | null>(null);

  /** Run an action, showing what happened under the steps. */
  const act = async (
    name: string,
    work: () => Promise<string | void>,
  ): Promise<void> => {
    setBusy(name);
    setMessage(null);
    try {
      const done = app.demo ? undefined : await work();
      if (done) setMessage({ kind: "ok", text: done });
    } catch (cause) {
      setMessage({ kind: "error", text: errorText(cause) });
    } finally {
      setBusy(null);
      await app.refresh();
    }
  };

  const mirrorPhase = app.mirror?.phase ?? "off";
  const btPhase = app.bluetooth?.phase ?? "unsupported";
  const mac = app.bluetooth?.macName ?? "your Mac’s name";
  const screenCopy = mirrorCopy(mirrorPhase);
  const btCopy = phaseCopy(btPhase, mac);

  const screenDone = mirrorPhase === "live" || mirrorPhase === "dark";
  const btOn = ["waitingForPhone", "pairing", "connected"].includes(btPhase);
  const paired = app.bluetoothConnected;
  const screenState: StepState = screenDone ? "done" : "current";
  const bluetoothState: StepState = btOn
    ? "done"
    : screenDone
      ? "current"
      : "todo";
  const pairState: StepState = paired ? "done" : btOn ? "current" : "todo";
  // Bluetooth doesn't wait for the cable: with the screen step unfinished,
  // its button still works.
  const allDone = screenDone && paired;

  const showScreen = () => act("screen", () => startMirror());
  const turnOnBluetooth = () => act("bluetooth", () => startBluetooth());
  const openBluetooth = () => act("btSettings", () => openBluetoothSettings());
  const tryGesture = (label: string, gesture: Gesture) =>
    act(label, async () => {
      await app.send(gesture);
      return `Sent: ${label}`;
    });

  const tests: { label: string; gesture: Gesture }[] = [
    {
      label: "Tap the center",
      gesture: {
        type: "tap",
        x: Math.round(app.screen.width / 2),
        y: Math.round(app.screen.height / 2),
      },
    },
    { label: "Go Home", gesture: { type: "press_home" } },
    { label: "Swipe up", gesture: edgeSwipe("up", app.screen) },
    { label: "Type “hello”", gesture: { type: "type_text", text: "hello" } },
  ];

  return (
    <div className="page">
      <header className="page-head">
        <span className="eyebrow">Setup · About 2 minutes</span>
        <h1>
          Give your agent
          <br />
          your iPhone<span className="dim">.</span>
        </h1>
        <p className="lede">
          Edart watches the screen over a USB cable and taps through Bluetooth.
          Nothing is installed on the iPhone.
        </p>
      </header>

      <ol className="setup-steps">
        <StepCard
          n={1}
          state={screenState}
          kicker="Step 1 · Camera access"
          title={
            screenDone ? "The iPhone’s screen is showing" : screenCopy.title
          }
          detail={
            screenDone
              ? "Over the USB cable. macOS lists the iPhone as a camera, which is why it asked for Camera access."
              : screenCopy.detail
          }
          error={app.mirror?.error}
        >
          {mirrorPhase === "off" && (
            <button
              type="button"
              className="primary"
              disabled={busy !== null}
              onClick={() => void showScreen()}
            >
              Show the screen
            </button>
          )}
          {mirrorPhase === "denied" && (
            <button
              type="button"
              className="primary"
              disabled={busy !== null}
              onClick={() => void act("camera", () => openCameraSettings())}
            >
              Open Camera settings
            </button>
          )}
        </StepCard>

        <StepCard
          n={2}
          state={bluetoothState}
          kicker="Step 2 · Bluetooth access"
          title={
            btOn ? "Edart is a Bluetooth mouse and keyboard" : btCopy.title
          }
          detail={
            btOn
              ? `Your iPhone lists it as “${mac}”.`
              : btPhase === "notStarted"
                ? "Edart taps and types on the iPhone as a Bluetooth mouse and keyboard. macOS asks for Bluetooth access once."
                : btCopy.detail
          }
          error={btPhase === "broken" ? app.bluetooth?.error : null}
        >
          {btPhase === "notStarted" && (
            <button
              type="button"
              className={bluetoothState === "current" ? "primary" : "secondary"}
              disabled={busy !== null}
              onClick={() => void turnOnBluetooth()}
            >
              Turn on
            </button>
          )}
          {btPhase === "needsPermission" && (
            <button
              type="button"
              className="primary"
              disabled={busy !== null}
              onClick={() => void openBluetooth()}
            >
              Open Bluetooth settings
            </button>
          )}
        </StepCard>

        <StepCard
          n={3}
          state={pairState}
          kicker="Step 3 · On the iPhone"
          title={paired ? "iPhone paired" : "Turn on AssistiveTouch and pair"}
          detail={
            paired
              ? "Edart can tap, swipe and type on your iPhone."
              : `Open Bluetooth settings on this Mac and leave them open. On the iPhone: Settings › Accessibility › Touch › AssistiveTouch, switch it on, then Devices › Bluetooth Devices and tap “${mac}”. Accept Pair on both.`
          }
        >
          {!paired && (
            <button
              type="button"
              className={pairState === "current" ? "primary" : "secondary"}
              disabled={busy !== null || !btOn}
              onClick={() => void openBluetooth()}
            >
              Open Bluetooth settings
            </button>
          )}
        </StepCard>
      </ol>

      {allDone && (
        <button
          type="button"
          className="primary wide"
          onClick={() => go("control")}
        >
          Open Control
        </button>
      )}
      {message && (
        <p
          className={message.kind === "error" ? "fine error" : "fine ok"}
          role="status"
        >
          {message.text}
        </p>
      )}

      <section className="group" aria-labelledby="permissions-title">
        <h2 id="permissions-title" className="eyebrow">
          Permissions
        </h2>
        <div className="card list">
          <div className="list-row">
            <span className="glyph" aria-hidden="true">
              <CameraIcon />
            </span>
            <span className="row-text">
              <strong>Camera</strong>
              <span>To see the iPhone’s screen over USB</span>
            </span>
            {app.mirror?.access === "allowed" ? (
              <span className="state good">Allowed</span>
            ) : mirrorPhase === "denied" ? (
              <button
                type="button"
                className="secondary small"
                onClick={() => void act("camera", () => openCameraSettings())}
              >
                Open settings
              </button>
            ) : mirrorPhase === "unsupported" ? (
              <span className="state">Needs a Mac</span>
            ) : (
              <button
                type="button"
                className="primary small"
                disabled={busy !== null || mirrorPhase === "needsPermission"}
                onClick={() => void showScreen()}
              >
                Allow
              </button>
            )}
          </div>
          <div className="list-row">
            <span className="glyph" aria-hidden="true">
              <BluetoothIcon />
            </span>
            <span className="row-text">
              <strong>Bluetooth</strong>
              <span>To tap, swipe and type on the iPhone</span>
            </span>
            {btPhase === "unsupported" ? (
              <span className="state">Needs a Mac</span>
            ) : btPhase === "notStarted" ? (
              <button
                type="button"
                className="primary small"
                disabled={busy !== null}
                onClick={() => void turnOnBluetooth()}
              >
                Allow
              </button>
            ) : btPhase === "needsPermission" ? (
              <button
                type="button"
                className="secondary small"
                onClick={() => void openBluetooth()}
              >
                Open settings
              </button>
            ) : (
              <span className="state good">Allowed</span>
            )}
          </div>
        </div>
      </section>

      <section className="group" aria-labelledby="try-title">
        <h2 id="try-title" className="eyebrow">
          Try it
        </h2>
        <div className="card tests">
          {tests.map((t) => (
            <button
              key={t.label}
              type="button"
              className="secondary"
              disabled={!app.controlReady || busy !== null}
              onClick={() => void tryGesture(t.label, t.gesture)}
            >
              {busy === t.label ? "Sending…" : t.label}
            </button>
          ))}
        </div>
        <p className="fine">
          {app.controlReady
            ? "A pointer appears on the iPhone. Typing needs a text field with the keyboard showing."
            : "These work once your iPhone is paired."}
        </p>
      </section>

      <details className="card details">
        <summary>Trouble pairing?</summary>
        <ul className="notes">
          <li>
            Keep this Mac’s Bluetooth settings open while pairing. Without them,
            macOS quietly refuses the iPhone’s request.
          </li>
          <li>
            The iPhone shows this Mac’s name, not “Edart”, and pairing from
            Settings › Bluetooth on the iPhone won’t work: use AssistiveTouch ›
            Devices.
          </li>
          <li>
            If it already says Connected but nothing happens, tap ⓘ › Forget
            This Device on the iPhone, then pick the Mac again.
          </li>
          <li>
            Bluetooth control does not work when the iPhone and this Mac are
            signed in to the same Apple Account.
          </li>
        </ul>
        <div className="button-row">
          <button
            type="button"
            className="secondary small"
            disabled={busy !== null || !btOn}
            onClick={() =>
              void act("restart", async () => {
                await restartBluetooth();
                return "Bluetooth services restarted. Try pairing again.";
              })
            }
          >
            Restart Bluetooth
          </button>
          <button
            type="button"
            className="secondary small"
            disabled={busy !== null || !btOn}
            onClick={() =>
              void act("reach", async () => {
                await reachOut();
                return "Asked the Mac to connect to your iPhone. Check Details.";
              })
            }
          >
            Connect from Mac
          </button>
        </div>
      </details>

      <Details app={app} />
    </div>
  );
}

function StepCard({
  n,
  state,
  kicker,
  title,
  detail,
  error,
  children,
}: {
  n: number;
  state: StepState;
  kicker: string;
  title: string;
  detail: string;
  error?: string | null;
  children?: ReactNode;
}) {
  return (
    <li className={`step-card ${state}`}>
      <span className="step-badge" aria-hidden="true">
        {state === "done" ? <CheckIcon size={22} /> : n}
      </span>
      <div className="step-copy">
        <span className="step-kicker">{kicker}</span>
        <strong>{title}</strong>
        <span>{detail}</span>
        {error && <span className="bad-text">{error}</span>}
      </div>
      {state === "done" ? <span className="state-pill">Done</span> : children}
    </li>
  );
}
