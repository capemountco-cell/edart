import { useEffect, useRef, useState, type FormEvent } from "react";

import { Chat } from "../components/Chat";
import { HomeIcon, KeyboardIcon, MoreIcon } from "../components/icons";
import { Mirror } from "../components/Mirror";
import { startMirror, type Gesture } from "../lib/native";
import { linkLabel } from "../lib/phase";
import { edgeSwipe, GESTURE_MODES, type GestureMode } from "../lib/pointer";
import { demoParam } from "../lib/demo";
import { errorText, type Edart } from "../lib/useEdart";
import type { Route } from "../App";

const NAMES: Record<Gesture["type"], string> = {
  tap: "Tap",
  double_tap: "Double tap",
  long_press: "Tap & hold",
  swipe: "Swipe",
  type_text: "Typed",
  press_home: "Home",
};

/**
 * The main screen: the phone, the buttons around it, and the agent.
 * Wide windows put the agent beside the phone; narrow ones put it in a
 * sheet behind the "Ask agent" button.
 */
export function Control({
  app,
  narrow,
  go,
}: {
  app: Edart;
  narrow: boolean;
  go: (route: Route) => void;
}) {
  const [mode, setMode] = useState<GestureMode>("live");
  const [tray, setTray] = useState<"type" | "more" | null>(null);
  const [text, setText] = useState("");
  const [note, setNote] = useState<{
    kind: "ok" | "error";
    text: string;
  } | null>(null);
  const [sheet, setSheet] = useState(
    () => app.demo && demoParam("sheet") === "agent",
  );
  const noteTimer = useRef(0);
  useEffect(() => () => window.clearTimeout(noteTimer.current), []);

  const say = (kind: "ok" | "error", message: string) => {
    setNote({ kind, text: message });
    window.clearTimeout(noteTimer.current);
    noteTimer.current = window.setTimeout(
      () => setNote(null),
      kind === "ok" ? 1800 : 6000,
    );
  };

  const run = async (gesture: Gesture) => {
    try {
      await app.send(gesture);
      say("ok", `Sent: ${NAMES[gesture.type]}`);
    } catch (cause) {
      say("error", errorText(cause));
    }
  };

  const type = async (event: FormEvent) => {
    event.preventDefault();
    if (text === "") return;
    await run({ type: "type_text", text });
    setText("");
  };

  const showScreen = async () => {
    try {
      await startMirror();
      await app.refresh();
    } catch (cause) {
      say("error", errorText(cause));
    }
  };

  const toggle = (which: "type" | "more") =>
    setTray((open) => (open === which ? null : which));
  const hint = GESTURE_MODES.find((m) => m.id === mode)?.hint ?? "";
  const links = linkLabel({
    mirror: app.mirrorHasPhone,
    usbControl: app.usbReady,
    bluetooth: app.bluetoothConnected,
  });
  const eyebrow = [app.usb?.iosVersion && `iOS ${app.usb.iosVersion}`, links]
    .filter(Boolean)
    .join(" · ");

  const chat = (inSheet: boolean) => (
    <Chat
      agent={app.agent}
      blocker={app.agentBlocker}
      onAsk={app.ask}
      onStop={app.stop}
      onNewChat={app.newChat}
      onFix={go}
      sheet={inSheet}
    />
  );

  const buttons = (
    <>
      <button
        type="button"
        className="tool"
        aria-label={narrow ? "Home" : undefined}
        onClick={() => void run({ type: "press_home" })}
      >
        <HomeIcon />
        <span>Home</span>
      </button>
      <button
        type="button"
        className="tool"
        aria-label={narrow ? "Type on iPhone" : undefined}
        aria-expanded={tray === "type"}
        onClick={() => toggle("type")}
      >
        <KeyboardIcon />
        <span>Type</span>
      </button>
    </>
  );
  const more = (
    <button
      type="button"
      className="tool"
      aria-label={narrow ? "More" : undefined}
      aria-expanded={tray === "more"}
      onClick={() => toggle("more")}
    >
      <MoreIcon />
      <span>More</span>
    </button>
  );

  return (
    <div className={`control ${narrow ? "narrow" : ""}`}>
      <section className="stage" aria-label="iPhone">
        {!narrow && (
          <header className="stage-head">
            <div>
              <span className="eyebrow">{eyebrow}</span>
              <h1 className="stage-title">{app.phoneName}</h1>
            </div>
            <StatusPill app={app} />
          </header>
        )}

        <div className="stage-body">
          <Mirror
            mirror={app.mirror}
            demo={app.demo}
            mode={mode}
            screen={app.screen}
            onGesture={(gesture) => void run(gesture)}
            onStart={() => void showScreen()}
            onOpenSetup={() => go("setup")}
          />
          {!narrow && (
            <div className="toolbar" role="toolbar" aria-label="Phone buttons">
              {buttons}
              {more}
            </div>
          )}
        </div>

        <div className="modes" role="radiogroup" aria-label="Gesture mode">
          {GESTURE_MODES.map((m) => (
            <button
              key={m.id}
              type="button"
              role="radio"
              aria-checked={mode === m.id}
              className="mode"
              onClick={() => setMode(m.id)}
            >
              {m.label}
            </button>
          ))}
        </div>

        {tray === "type" && (
          <form className="tray" onSubmit={(e) => void type(e)}>
            <label htmlFor="type-text" className="sr-only">
              Text to type on the iPhone
            </label>
            <input
              id="type-text"
              className="field"
              autoFocus
              autoComplete="off"
              spellCheck={false}
              placeholder="Type on the iPhone…"
              value={text}
              onChange={(e) => setText(e.target.value)}
            />
            <button
              type="submit"
              className="primary small"
              disabled={text === ""}
            >
              Type
            </button>
          </form>
        )}
        {tray === "more" && (
          <div className="tray" role="group" aria-label="Swipes">
            {(["up", "down", "left", "right"] as const).map((direction) => (
              <button
                key={direction}
                type="button"
                className="secondary small"
                onClick={() => void run(edgeSwipe(direction, app.screen))}
              >
                Swipe {direction}
              </button>
            ))}
          </div>
        )}

        <p
          className={`hint ${note?.kind === "error" ? "error" : ""}`}
          role="status"
        >
          {note
            ? note.text
            : tray === "type"
              ? "Tap a text field on the iPhone first, so its keyboard is showing."
              : app.controlReady
                ? hint
                : "Not paired yet: clicks won’t reach the iPhone. Pair it in Permissions."}
        </p>
      </section>

      {narrow ? (
        <>
          <div className="dock" role="toolbar" aria-label="Phone controls">
            {buttons}
            <button
              type="button"
              className="ask"
              onClick={() => setSheet(true)}
            >
              {app.agent.running ? "Agent working…" : "Ask agent"}
            </button>
            {more}
          </div>
          {sheet && (
            <div className="sheet-layer">
              <button
                type="button"
                className="sheet-scrim"
                aria-label="Close agent"
                onClick={() => setSheet(false)}
              />
              <div className="sheet" role="dialog" aria-label="Agent">
                <span className="sheet-grip" aria-hidden="true" />
                {chat(true)}
              </div>
            </div>
          )}
        </>
      ) : (
        <aside className="chat-panel" aria-label="Agent chat">
          {chat(false)}
        </aside>
      )}
    </div>
  );
}

/** "Connected", or what is still missing, in one pill. */
export function StatusPill({ app, name }: { app: Edart; name?: boolean }) {
  const state = app.controlReady ? "ok" : app.mirrorHasPhone ? "warn" : "idle";
  const label = name
    ? app.phoneName
    : state === "ok"
      ? "Connected"
      : state === "warn"
        ? "Screen only"
        : "Not connected";
  return (
    <span className={`pill pill-${state}`}>
      <span className="pill-dot" aria-hidden="true" />
      {label}
    </span>
  );
}
