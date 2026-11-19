import { useEffect, useRef, useState, type PointerEvent } from "react";

import {
  frameUrl,
  type Gesture,
  type MirrorStatus,
  type Screen,
} from "../lib/native";
import { mirrorCopy } from "../lib/phase";
import {
  gestureFor,
  type GestureMode,
  type PointerSample,
} from "../lib/pointer";
import { CableIcon } from "./icons";

/** Aim for about twelve frames a second. */
const FRAME_MS = 80;

/**
 * The phone: its live screen inside a frame, taking clicks and drags as
 * gestures. Without a picture it still takes them, on a blank screen with
 * an explanation, because Bluetooth control works without the cable.
 */
export function Mirror({
  mirror,
  demo,
  mode,
  screen,
  onGesture,
  onStart,
  onOpenSetup,
}: {
  mirror: MirrorStatus | null;
  demo: boolean;
  mode: GestureMode;
  screen: Screen;
  onGesture: (gesture: Gesture) => void;
  /** Switch the mirror on (asks for Camera access). */
  onStart: () => void;
  onOpenSetup: () => void;
}) {
  const phase = mirror?.phase ?? "off";
  const hasPicture = phase === "live" || phase === "dark";
  const canvas = useRef<HTMLCanvasElement>(null);
  const surface = useRef<HTMLDivElement>(null);
  const down = useRef<PointerSample | null>(null);
  const [ring, setRing] = useState<{ x: number; y: number } | null>(null);

  // Pull frames one after another while there is a picture to show.
  const streaming = hasPicture && !demo;
  useEffect(() => {
    if (!streaming) return;
    let stopped = false;
    let timer = 0;
    let n = 0;
    const next = () => {
      if (stopped) return;
      const started = performance.now();
      const image = new Image();
      image.onload = () => {
        if (stopped) return;
        const target = canvas.current;
        if (target) {
          if (target.width !== image.naturalWidth) {
            target.width = image.naturalWidth;
          }
          if (target.height !== image.naturalHeight) {
            target.height = image.naturalHeight;
          }
          target.getContext("2d")?.drawImage(image, 0, 0);
        }
        const spent = performance.now() - started;
        timer = window.setTimeout(next, Math.max(0, FRAME_MS - spent));
      };
      image.onerror = () => {
        timer = window.setTimeout(next, 500);
      };
      image.src = frameUrl(n++);
    };
    next();
    return () => {
      stopped = true;
      window.clearTimeout(timer);
    };
  }, [streaming]);

  const sample = (event: PointerEvent): PointerSample => ({
    x: event.clientX,
    y: event.clientY,
    t: event.timeStamp,
  });
  const ringAt = (event: PointerEvent) => {
    const box = surface.current?.getBoundingClientRect();
    if (box)
      setRing({ x: event.clientX - box.left, y: event.clientY - box.top });
  };

  const press = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    down.current = sample(event);
    ringAt(event);
  };
  const move = (event: PointerEvent<HTMLDivElement>) => {
    if (down.current) ringAt(event);
  };
  const release = (event: PointerEvent<HTMLDivElement>) => {
    const start = down.current;
    down.current = null;
    window.setTimeout(() => setRing(null), 180);
    const box = surface.current?.getBoundingClientRect();
    if (!start || !box) return;
    const gesture = gestureFor(mode, start, sample(event), box, screen);
    if (gesture) onGesture(gesture);
  };
  const cancel = () => {
    down.current = null;
    setRing(null);
  };

  const copy = mirrorCopy(phase);
  const landscape = hasPicture && screen.width > screen.height;

  return (
    <div
      className={`phone ${landscape ? "landscape" : ""}`}
      style={
        { "--ar": `${screen.width} / ${screen.height}` } as React.CSSProperties
      }
    >
      <div
        ref={surface}
        className="phone-screen"
        role="application"
        aria-label={
          hasPicture
            ? "Live mirror of the iPhone screen. Click and drag to control it."
            : "The iPhone screen. No picture yet; clicks still reach the iPhone."
        }
        onPointerDown={press}
        onPointerMove={move}
        onPointerUp={release}
        onPointerCancel={cancel}
      >
        {demo && hasPicture ? (
          <DemoScreen />
        ) : hasPicture ? (
          <canvas ref={canvas} className="phone-canvas" />
        ) : null}

        {!hasPicture && (
          <div className="phone-empty">
            <span className="phone-empty-icon">
              <CableIcon size={24} />
            </span>
            <strong>{copy.title}</strong>
            <span>{copy.detail}</span>
            {mirror?.error && <span className="bad-text">{mirror.error}</span>}
            {phase === "off" && (
              <button
                type="button"
                className="primary small"
                onPointerDown={(e) => e.stopPropagation()}
                onClick={onStart}
              >
                Show the screen
              </button>
            )}
            {(phase === "denied" || phase === "stalled") && (
              <button
                type="button"
                className="secondary small"
                onPointerDown={(e) => e.stopPropagation()}
                onClick={onOpenSetup}
              >
                Open Permissions
              </button>
            )}
          </div>
        )}
        {phase === "dark" && (
          <div className="phone-dark">
            <strong>{copy.title}</strong>
            <span>{copy.detail}</span>
          </div>
        )}
        {ring && (
          <span
            className="touch-ring"
            style={{ left: ring.x, top: ring.y }}
            aria-hidden="true"
          />
        )}
      </div>
    </div>
  );
}

/** A drawn Home Screen, standing in for the phone in `?demo`. */
function DemoScreen() {
  const shades = [16, 10, 13, 8, 9, 15, 10, 12, 12, 8, 14, 10, 11, 9, 13, 10];
  const dock = [16, 12, 14, 10];
  const fill = (shade: number) => `rgba(255,255,255,${shade / 100})`;
  return (
    <svg
      className="demo-screen"
      viewBox="0 0 390 844"
      preserveAspectRatio="none"
      aria-hidden="true"
    >
      <defs>
        <radialGradient id="demo-a" cx="0.2" cy="0" r="0.8">
          <stop offset="0" stopColor="rgba(96,165,250,0.28)" />
          <stop offset="1" stopColor="rgba(96,165,250,0)" />
        </radialGradient>
        <radialGradient id="demo-b" cx="1" cy="1" r="0.8">
          <stop offset="0" stopColor="rgba(244,114,182,0.22)" />
          <stop offset="1" stopColor="rgba(244,114,182,0)" />
        </radialGradient>
      </defs>
      <rect width="390" height="844" fill="#0b0b0c" />
      <rect width="390" height="844" fill="url(#demo-a)" />
      <rect width="390" height="844" fill="url(#demo-b)" />
      <text x="44" y="48" fill="#fff" fontSize="17" fontWeight="600">
        9:41
      </text>
      <rect x="132" y="22" width="126" height="36" rx="18" fill="#000" />
      <rect
        x="320"
        y="34"
        width="27"
        height="13"
        rx="4"
        fill="#fff"
        opacity="0.85"
      />
      {shades.map((shade, i) => (
        <rect
          key={i}
          x={32 + (i % 4) * 86}
          y={96 + Math.floor(i / 4) * 96}
          width="68"
          height="68"
          rx="16"
          fill={fill(shade)}
        />
      ))}
      <rect x="16" y="716" width="358" height="96" rx="34" fill={fill(10)} />
      {dock.map((shade, i) => (
        <rect
          key={i}
          x={32 + i * 86}
          y={730}
          width="68"
          height="68"
          rx="16"
          fill={fill(shade)}
        />
      ))}
      <rect
        x="125"
        y="826"
        width="140"
        height="5"
        rx="2.5"
        fill="#fff"
        opacity="0.7"
      />
    </svg>
  );
}
