/** Money as the Trading screen shows and reads it. Amounts are cents. */

/** 10 → "$0.10". */
export function formatCents(cents: number): string {
  const whole = Math.floor(cents / 100);
  return `$${whole}.${String(cents % 100).padStart(2, "0")}`;
}

/** 10 → "0.10", for a field the user edits. */
export const centsField = (cents: number) => formatCents(cents).slice(1);

/**
 * What the user typed as dollars, in cents: "0.10", "$1", ".5", "2.50".
 * `null` when it is not an amount (or has more than two decimals), so the
 * field can say so instead of guessing.
 */
export function parseDollars(text: string): number | null {
  const match = /^\$?(\d{0,7})(?:\.(\d{0,2}))?$/.exec(text.trim());
  if (!match) return null;
  const [, whole = "", cents = ""] = match;
  if (whole === "" && cents === "") return null;
  return Number(whole || "0") * 100 + Number(cents.padEnd(2, "0") || "0");
}

/** What the user typed as a whole number, or `null`. */
export function parseCount(text: string): number | null {
  return /^\d{1,6}$/.test(text.trim()) ? Number(text.trim()) : null;
}
