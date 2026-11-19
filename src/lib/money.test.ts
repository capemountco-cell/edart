import { describe, expect, it } from "vitest";

import { centsField, formatCents, parseCount, parseDollars } from "./money";

describe("money", () => {
  it("writes cents as dollars", () => {
    expect(formatCents(10)).toBe("$0.10");
    expect(formatCents(100)).toBe("$1.00");
    expect(formatCents(1586)).toBe("$15.86");
    expect(formatCents(0)).toBe("$0.00");
    expect(centsField(250)).toBe("2.50");
  });

  it("reads dollars exactly, or not at all", () => {
    expect(parseDollars("0.10")).toBe(10);
    expect(parseDollars("$0.1")).toBe(10);
    expect(parseDollars(".5")).toBe(50);
    expect(parseDollars(" 1 ")).toBe(100);
    expect(parseDollars("2.50")).toBe(250);
    expect(parseDollars("100")).toBe(10000);
    expect(parseDollars("0.07")).toBe(7);
    for (const bad of ["", ".", "$", "0.105", "1,00", "ten", "-1", "1e2"]) {
      expect(parseDollars(bad)).toBeNull();
    }
  });

  it("reads whole numbers only", () => {
    expect(parseCount("10")).toBe(10);
    expect(parseCount(" 3 ")).toBe(3);
    for (const bad of ["", "1.5", "-2", "ten"]) {
      expect(parseCount(bad)).toBeNull();
    }
  });
});
