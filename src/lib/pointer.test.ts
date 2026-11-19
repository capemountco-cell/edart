import { describe, expect, it } from "vitest";

import { edgeSwipe, gestureFor, toScreen } from "./pointer";

/** A 300x650 mirror at (100, 50) showing a 739x1600 frame. */
const box = { left: 100, top: 50, width: 300, height: 650 };
const screen = { width: 739, height: 1600 };
const at = (x: number, y: number, t = 0) => ({ x, y, t });

describe("toScreen", () => {
  it("maps window pixels to frame pixels by proportion", () => {
    expect(toScreen({ x: 100, y: 50 }, box, screen)).toEqual({ x: 0, y: 0 });
    expect(toScreen({ x: 400, y: 700 }, box, screen)).toEqual({
      x: 739,
      y: 1600,
    });
    expect(toScreen({ x: 250, y: 375 }, box, screen)).toEqual({
      x: 370,
      y: 800,
    });
  });

  it("keeps a point dragged past the edge on the screen", () => {
    expect(toScreen({ x: 20, y: 900 }, box, screen)).toEqual({
      x: 0,
      y: 1600,
    });
  });
});

describe("gestureFor", () => {
  it("reads a quick click as a tap in Live and Tap modes", () => {
    const tap = { type: "tap", x: 370, y: 800 };
    expect(
      gestureFor("live", at(250, 375), at(251, 376, 90), box, screen),
    ).toEqual(tap);
    expect(
      gestureFor("tap", at(250, 375), at(250, 375, 900), box, screen),
    ).toEqual(tap);
  });

  it("reads a held click as a long press in Live mode", () => {
    expect(
      gestureFor("live", at(250, 375), at(250, 375, 700), box, screen),
    ).toEqual({ type: "long_press", x: 370, y: 800, ms: 700 });
  });

  it("reads a drag as a swipe that takes as long as the drag", () => {
    expect(
      gestureFor("live", at(250, 600), at(250, 200, 400), box, screen),
    ).toEqual({
      type: "swipe",
      from_x: 370,
      from_y: 1354,
      to_x: 370,
      to_y: 369,
      ms: 400,
    });
  });

  it("bounds swipe durations", () => {
    const fast = gestureFor("live", at(250, 600), at(250, 200, 5), box, screen);
    expect(fast).toMatchObject({ type: "swipe", ms: 120 });
    const slow = gestureFor(
      "drag",
      at(250, 600),
      at(250, 200, 60_000),
      box,
      screen,
    );
    expect(slow).toMatchObject({ type: "swipe", ms: 3000 });
  });

  it("uses the press point for the fixed modes, even after a drag", () => {
    expect(
      gestureFor("double", at(250, 375), at(300, 500, 200), box, screen),
    ).toEqual({ type: "double_tap", x: 370, y: 800 });
    expect(
      gestureFor("hold", at(250, 375), at(250, 375, 50), box, screen),
    ).toEqual({ type: "long_press", x: 370, y: 800 });
  });

  it("makes Flick fast and Drag slow, and ignores a click in either", () => {
    expect(
      gestureFor("flick", at(250, 600), at(250, 200, 900), box, screen),
    ).toMatchObject({ type: "swipe", ms: 150 });
    expect(
      gestureFor("drag", at(250, 600), at(250, 200, 100), box, screen),
    ).toMatchObject({ type: "swipe", ms: 500 });
    expect(
      gestureFor("flick", at(250, 600), at(251, 601, 100), box, screen),
    ).toBeNull();
    expect(
      gestureFor("drag", at(250, 600), at(250, 600, 100), box, screen),
    ).toBeNull();
  });

  it("does nothing without a mirror on screen", () => {
    const empty = { left: 0, top: 0, width: 0, height: 0 };
    expect(gestureFor("tap", at(1, 1), at(1, 1), empty, screen)).toBeNull();
  });
});

describe("edgeSwipe", () => {
  it("swipes across the middle, inside the screen", () => {
    expect(edgeSwipe("up", screen)).toEqual({
      type: "swipe",
      from_x: 370,
      from_y: 1200,
      to_x: 370,
      to_y: 400,
      ms: 300,
    });
    const left = edgeSwipe("left", screen);
    expect(left).toMatchObject({ from_x: 592, to_x: 148, from_y: 800 });
  });
});
