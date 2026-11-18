import { describe, expect, it } from "vitest";

import { bluetoothStatus, nativeAvailable, perform } from "./native";

describe("native bridge", () => {
  it("reports that it is not inside the Tauri window under test", () => {
    expect(nativeAvailable()).toBe(false);
  });

  it("refuses with a useful message instead of hanging", async () => {
    await expect(bluetoothStatus()).rejects.toThrow(/npm run dev/);
    await expect(
      perform({ type: "press_home" }, { width: 1, height: 1 }),
    ).rejects.toThrow(/npm run dev/);
  });
});
