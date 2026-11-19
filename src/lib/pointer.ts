import type { Gesture, Screen } from "./native";

/** How a click or drag on the mirror is read. */
export type GestureMode = "live" | "tap" | "double" | "hold" | "flick" | "drag";

export const GESTURE_MODES: { id: GestureMode; label: string; hint: string }[] =
  [
    {
      id: "live",
      label: "Live",
      hint: "Live: click to tap, drag to swipe, hold to press.",
    },
    { id: "tap", label: "Tap", hint: "Tap: each click is one tap." },
    {
      id: "double",
      label: "Double tap",
      hint: "Double tap: each click taps twice.",
    },
    {
      id: "hold",
      label: "Tap & hold",
      hint: "Tap & hold: each click presses and holds.",
    },
    {
      id: "flick",
      label: "Flick",
      hint: "Flick: drag for a quick swipe that keeps scrolling.",
    },
    {
      id: "drag",
      label: "Drag",
      hint: "Drag: press, move slowly, then let go.",
    },
  ];

/** Where and when the pointer went down or up, in window pixels. */
export interface PointerSample {
  x: number;
  y: number;
  /** Milliseconds, from any steady clock. */
  t: number;
}

/** The mirror's on-screen rectangle (as from `getBoundingClientRect`). */
export interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

/** Moving less than this many window pixels is a click, not a drag. */
export const DRAG_THRESHOLD_PX = 8;
/** In Live mode, a click held this long is a long press. */
export const HOLD_MS = 500;

const clamp = (value: number, low: number, high: number) =>
  Math.min(high, Math.max(low, value));

/** A window point as a point on the phone's screen, kept on the screen. */
export function toScreen(
  point: { x: number; y: number },
  box: Box,
  screen: Screen,
): { x: number; y: number } {
  const x = ((point.x - box.left) / box.width) * screen.width;
  const y = ((point.y - box.top) / box.height) * screen.height;
  return {
    x: Math.round(clamp(x, 0, screen.width)),
    y: Math.round(clamp(y, 0, screen.height)),
  };
}

/**
 * The gesture a press-and-release on the mirror means in a mode, or null
 * when it means nothing (a Flick or Drag with no movement, an empty box).
 */
export function gestureFor(
  mode: GestureMode,
  down: PointerSample,
  up: PointerSample,
  box: Box,
  screen: Screen,
): Gesture | null {
  if (!(box.width > 0 && box.height > 0)) return null;
  const from = toScreen(down, box, screen);
  const to = toScreen(up, box, screen);
  const moved =
    Math.hypot(up.x - down.x, up.y - down.y) >= DRAG_THRESHOLD_PX &&
    (from.x !== to.x || from.y !== to.y);
  const held = Math.max(0, up.t - down.t);
  const swipe = (ms: number): Gesture => ({
    type: "swipe",
    from_x: from.x,
    from_y: from.y,
    to_x: to.x,
    to_y: to.y,
    ms: Math.round(ms),
  });

  switch (mode) {
    case "tap":
      return { type: "tap", ...from };
    case "double":
      return { type: "double_tap", ...from };
    case "hold":
      return { type: "long_press", ...from };
    case "flick":
      return moved ? swipe(150) : null;
    case "drag":
      return moved ? swipe(clamp(held, 500, 3000)) : null;
    case "live":
      if (moved) return swipe(clamp(held, 120, 1500));
      if (held >= HOLD_MS)
        return { type: "long_press", ...from, ms: clamp(held, 500, 3000) };
      return { type: "tap", ...from };
  }
}

/** A swipe across the middle of the screen, for the More menu. */
export function edgeSwipe(
  direction: "up" | "down" | "left" | "right",
  screen: Screen,
): Gesture {
  const cx = Math.round(screen.width / 2);
  const cy = Math.round(screen.height / 2);
  const dx = Math.round(screen.width * 0.3);
  const dy = Math.round(screen.height * 0.25);
  const swipe = (
    from_x: number,
    from_y: number,
    to_x: number,
    to_y: number,
  ): Gesture => ({ type: "swipe", from_x, from_y, to_x, to_y, ms: 300 });
  switch (direction) {
    case "up":
      return swipe(cx, cy + dy, cx, cy - dy);
    case "down":
      return swipe(cx, cy - dy, cx, cy + dy);
    case "left":
      return swipe(cx + dx, cy, cx - dx, cy);
    case "right":
      return swipe(cx - dx, cy, cx + dx, cy);
  }
}
