import { describe, expect, it } from "vitest";

import type { UsbPhase } from "./native";
import { usbPhaseCopy, usbStepStates } from "./phase";

const PHASES: UsbPhase[] = [
  "unsupported",
  "hostMissing",
  "noDevice",
  "restarting",
  "untrusted",
  "locked",
  "oldIos",
  "developerModeOff",
  "developerModeManual",
  "confirmDeveloperMode",
  "xcodeMissing",
  "notPrepared",
  "preparing",
  "diskImageMissing",
  "serviceUnavailable",
  "ready",
];

describe("USB setup copy", () => {
  it("has a headline and a next step for every phase", () => {
    for (const phase of PHASES) {
      const copy = usbPhaseCopy(phase, "iPhone Air");
      expect(copy.title.length, phase).toBeGreaterThan(0);
      expect(copy.detail.length, phase).toBeGreaterThan(0);
    }
  });

  it("names the phone", () => {
    expect(usbPhaseCopy("ready", "iPhone Air").detail).toContain("iPhone Air");
  });
});

describe("USB setup steps", () => {
  it("has exactly one current step until everything is done", () => {
    for (const phase of PHASES) {
      const current = usbStepStates(phase).filter((s) => s === "current");
      expect(current.length, phase).toBe(phase === "ready" ? 0 : 1);
    }
  });

  it("walks forward through the setup", () => {
    expect(usbStepStates("noDevice")).toEqual([
      "current",
      "todo",
      "todo",
      "todo",
    ]);
    expect(usbStepStates("developerModeOff")[1]).toBe("current");
    expect(usbStepStates("restarting")).toEqual([
      "done",
      "done",
      "current",
      "todo",
    ]);
    expect(usbStepStates("notPrepared")[3]).toBe("current");
    expect(usbStepStates("ready")).toEqual(["done", "done", "done", "done"]);
  });
});
