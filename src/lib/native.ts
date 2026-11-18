/**
 * Every call from the UI to Rust goes through this file, so the command
 * surface is visible in one place and argument shapes cannot drift.
 */

import { invoke } from "@tauri-apps/api/core";

/** True inside the Tauri window; false in a plain browser tab. */
export function nativeAvailable(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function call<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (!nativeAvailable()) {
    throw new Error(
      "The native side is not running. Start the app with `npm run dev`.",
    );
  }
  return invoke<T>(command, args);
}

/** Mirrors `edart_core::status::Phase`. */
export type Phase =
  | "unsupported"
  | "notStarted"
  | "needsPermission"
  | "bluetoothOff"
  | "starting"
  | "broken"
  | "waitingForPhone"
  | "pairing"
  | "connected";

/** Mirrors `edart_core::status::StatusView`. */
export interface BluetoothStatus {
  phase: Phase;
  supported: boolean;
  state: string;
  published: boolean;
  advertising: boolean;
  subscribed: boolean;
  phoneSeen: boolean;
  macName: string | null;
  failedServices: string[];
  error: string | null;
  /** Recent Bluetooth events, oldest first. */
  events: string[];
}

/** Mirrors `edart_core::gesture::Screen`. */
export interface Screen {
  width: number;
  height: number;
}

/** Mirrors `edart_core::gesture::Gesture` (serde tag "type"). */
export type Gesture =
  | { type: "tap"; x: number; y: number }
  | { type: "double_tap"; x: number; y: number }
  | { type: "long_press"; x: number; y: number; ms?: number }
  | {
      type: "swipe";
      from_x: number;
      from_y: number;
      to_x: number;
      to_y: number;
      ms?: number;
    }
  | { type: "type_text"; text: string }
  | { type: "press_home" };

export const bluetoothStatus = () => call<BluetoothStatus>("bt_status");
export const startBluetooth = () => call<void>("bt_start");
export const restartBluetooth = () => call<void>("bt_restart");
export const reachOut = () => call<void>("bt_reach_out");
export const openBluetoothSettings = () =>
  call<void>("open_bluetooth_settings");
export const perform = (gesture: Gesture, screen: Screen) =>
  call<void>("perform", { gesture, screen });
