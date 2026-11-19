/**
 * Every call from the UI to Rust goes through this file, so the command
 * surface is visible in one place and argument shapes cannot drift.
 */

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

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

/** Mirrors `edart_core::usb::readiness::UsbPhase`. */
export type UsbPhase =
  | "unsupported"
  | "hostMissing"
  | "noDevice"
  | "restarting"
  | "untrusted"
  | "locked"
  | "oldIos"
  | "developerModeOff"
  | "developerModeManual"
  | "confirmDeveloperMode"
  | "xcodeMissing"
  | "notPrepared"
  | "preparing"
  | "diskImageMissing"
  | "serviceUnavailable"
  | "ready";

/** Mirrors `edart_core::usb::readiness::UsbStatus`. */
export interface UsbStatus {
  phase: UsbPhase;
  deviceName: string | null;
  iosVersion: string | null;
  developerMode: boolean | null;
  coreDeviceVersion: string | null;
  error: string | null;
  /** Recent USB events, oldest first. */
  events: string[];
}

/** Mirrors `edart_core::usb::readiness::DeveloperModeOutcome`. */
export type DeveloperModeOutcome = "restarting" | "useSettings";

export const usbStatus = () => call<UsbStatus>("usb_status");
/** Ask the plugged-in iPhone to turn Developer Mode on. */
export const usbEnableDeveloperMode = () =>
  call<DeveloperModeOutcome>("usb_enable_developer_mode");
/** Connect to the iPhone's developer services. Can take a minute. */
export const usbPrepare = () => call<void>("usb_prepare");

export const bluetoothStatus = () => call<BluetoothStatus>("bt_status");
export const startBluetooth = () => call<void>("bt_start");
export const restartBluetooth = () => call<void>("bt_restart");
export const reachOut = () => call<void>("bt_reach_out");
export const openBluetoothSettings = () =>
  call<void>("open_bluetooth_settings");
export const perform = (gesture: Gesture, screen: Screen) =>
  call<void>("perform", { gesture, screen });

/** Mirrors `edart_core::mirror::MirrorPhase`. */
export type MirrorPhase =
  | "unsupported"
  | "off"
  | "needsPermission"
  | "denied"
  | "noDevice"
  | "starting"
  | "stalled"
  | "dark"
  | "live";

/** Mirrors `edart_core::mirror::CameraAccess`. */
export type CameraAccess =
  | "unknown"
  | "notDetermined"
  | "denied"
  | "restricted"
  | "allowed";

/** Mirrors `edart_core::mirror::MirrorStatus`. */
export interface MirrorStatus {
  phase: MirrorPhase;
  access: CameraAccess;
  deviceName: string | null;
  /** Size of the mirrored frame; gestures use it as their screen units. */
  width: number | null;
  height: number | null;
  orientation: "portrait" | "landscape" | null;
  /** Goes up with every new frame. */
  seq: number;
  error: string | null;
  /** Recent mirror events, oldest first. */
  events: string[];
}

export const mirrorStatus = () => call<MirrorStatus>("mirror_status");
/** Start showing the iPhone's screen. Asks for Camera access once. */
export const startMirror = () => call<void>("mirror_start");
export const openCameraSettings = () => call<void>("open_camera_settings");
/**
 * Address of the newest mirror frame (a JPEG). `n` only makes each address
 * different, so the webview fetches it again.
 */
export const frameUrl = (n: number) =>
  `${convertFileSrc("frame.jpg", "edart")}?n=${n}`;

/** Mirrors `edart_core::settings::SettingsView`. The token is never sent. */
export interface SettingsView {
  tokenSet: boolean;
  apiBase: string;
  apiBaseIsDefault: boolean;
  /** Where the settings file is. */
  path: string;
  /** Sent to the agent with every step; empty when there are none. */
  instructions: string;
  /** The agent may type text of its own, not only what the user wrote. */
  allowTyping: boolean;
}

/**
 * Mirrors `edart_core::settings::SettingsChange`. A field left out is left
 * alone; an empty string clears it.
 */
export interface SettingsChange {
  token?: string;
  apiBase?: string;
  instructions?: string;
  allowTyping?: boolean;
}

/** Mirrors `edart_core::agent::ConnectionTest`. */
export interface ConnectionTest {
  reachable: boolean;
  token: "accepted" | "refused" | "serviceBroken" | "unexpected" | null;
  message: string;
}

export const getSettings = () => call<SettingsView>("settings_get");
export const setSettings = (change: SettingsChange) =>
  call<SettingsView>("settings_set", { change });
/** Check that the AI service answers and accepts the saved token. */
export const testSettings = () => call<ConnectionTest>("settings_test");

/** Mirrors `edart_core::agent::Entry` (serde tag "kind"). */
export type AgentEntry =
  | { kind: "goal"; text: string }
  | { kind: "step"; n: number; thought: string; action: string }
  | { kind: "problem"; text: string }
  | { kind: "finished"; success: boolean; text: string };

/** Mirrors `edart_core::agent::AgentStatus`. */
export interface AgentStatus {
  running: boolean;
  stopping: boolean;
  step: number;
  maxSteps: number;
  entries: AgentEntry[];
}

export const agentStatus = () => call<AgentStatus>("agent_status");
/** Start the agent on a goal. Rejects with a sentence when it can't. */
export const startAgent = (goal: string) => call<void>("agent_start", { goal });
export const stopAgent = () => call<void>("agent_stop");
/** Forget the conversation. */
export const clearAgent = () => call<void>("agent_clear");
/**
 * Call `handler` whenever the agent's status changes. Returns a function
 * that stops listening. Does nothing outside the Tauri window.
 */
export async function onAgentChanged(handler: () => void): Promise<() => void> {
  if (!nativeAvailable()) return () => {};
  return listen("agent-changed", () => handler());
}

/** Mirrors `edart_core::ledger::Counters`. */
export interface TradingCounters {
  /** The local date, "2026-11-19". */
  day: string;
  trades: number;
  spentCents: number;
}

/** Mirrors `edart_core::guard::BuyInfo`. */
export interface BuyInfo {
  side: "up" | "down" | null;
  stakeCents: number;
  market: string;
}

/** Mirrors `edart_core::trading::PendingBuy`. */
export interface PendingBuy {
  /** "Up, $0.10, ZEC 15 min". */
  summary: string;
  buy: BuyInfo;
  secondsLeft: number;
}

/** Mirrors `edart_core::trading::TradingView`. */
export interface TradingView {
  enabled: boolean;
  stakeCents: number;
  maxTradesPerDay: number;
  maxLossCentsPerDay: number;
  confirmEachBuy: boolean;
  maxSteps: number;
  /** The one app the agent may use. */
  app: string;
  /** Buys are allowed only in this market; empty is any market. */
  market: string;
  /** The most that can be lost in a day under these rules. */
  worstCaseCents: number;
  /** "Kalshi only · $0.10 · 3/10 buys · $0.30/$1.00". */
  summary: string;
  playbook: string;
  playbookIsDefault: boolean;
  today: TradingCounters;
  tradesLeft: number;
  /** Why the ledger can't be read; nothing can be bought until it can. */
  ledgerError: string | null;
  /** A buy waiting for the user's yes or no. */
  pending: PendingBuy | null;
}

/**
 * Mirrors `edart_core::trading::TradingChange`. A field left out is left
 * alone. Out-of-range values are refused with a sentence.
 */
export interface TradingChange {
  enabled?: boolean;
  stakeCents?: number;
  maxTradesPerDay?: number;
  maxLossCentsPerDay?: number;
  confirmEachBuy?: boolean;
  maxSteps?: number;
  app?: string;
  market?: string;
  /** Empty goes back to the built-in playbook. */
  playbook?: string;
}

/** Mirrors `edart_core::ledger::LedgerEvent`. */
export interface LedgerEvent {
  /** Seconds since 1970. */
  time: number;
  day: string;
  kind: "buy" | "sell" | "blocked";
  side?: "up" | "down";
  stakeCents: number;
  market?: string;
  reason?: string;
}

export const getTrading = () => call<TradingView>("trading_get");
/** Change the trading rules. They hold from the next gesture on. */
export const setTrading = (change: TradingChange) =>
  call<TradingView>("trading_set", { change });
/** The newest lines of the trading ledger, newest first. */
export const tradingLedger = () => call<LedgerEvent[]>("trading_ledger");
/** Answer the buy that is waiting for approval. */
export const confirmTrade = (approve: boolean) =>
  call<void>("trading_confirm", { approve });
/**
 * Call `handler` whenever the trading rules, the ledger or a pending
 * confirmation change. Returns a function that stops listening.
 */
export async function onTradingChanged(
  handler: () => void,
): Promise<() => void> {
  if (!nativeAvailable()) return () => {};
  return listen("trading-changed", () => handler());
}
