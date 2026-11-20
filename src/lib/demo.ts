/**
 * Sample data for `?demo`: every screen renders in a plain browser tab,
 * for screenshots and design work. Nothing here runs in the app.
 *
 * `?demo&screen=phones` picks a screen; `&chat=run` or `&chat=done` fills
 * the agent chat; `&mirror=off` shows the Control screen without a
 * mirror; `&sheet=agent` opens the agent sheet in a narrow window;
 * `&screen=trading` shows the Trading screen, `&trading=off` with the
 * rails off, and `&confirm=buy` with a buy waiting for approval.
 */
import type {
  AgentStatus,
  BluetoothStatus,
  LedgerEvent,
  MirrorStatus,
  SettingsView,
  TradingView,
  UsbStatus,
} from "./native";

const params = () =>
  typeof window === "undefined"
    ? new URLSearchParams()
    : new URLSearchParams(window.location.search);

export const isDemo = () => params().has("demo");
export const demoParam = (name: string) => params().get(name);

export const DEMO_BLUETOOTH: BluetoothStatus = {
  phase: "connected",
  supported: true,
  state: "poweredOn",
  published: true,
  advertising: true,
  subscribed: true,
  phoneSeen: true,
  paused: false,
  refreshing: false,
  macName: "Cape’s MacBook",
  failedServices: [],
  events: [
    "    0.0s  started",
    "    0.1s  Bluetooth poweredOn",
    "    0.2s  service added 00001812-0000-1000-8000-00805F9B34FB",
    "    0.3s  advertising",
    "   14.2s  subscribe 2A4D by 6F1C2A90",
  ],
  error: null,
};

export const DEMO_USB: UsbStatus = {
  phase: "developerModeOff",
  deviceName: "Cape’s iPhone",
  iosVersion: "27.3",
  developerMode: false,
  coreDeviceVersion: null,
  error: null,
  events: [
    "    0.0s  watching for an iPhone on USB",
    "    0.4s  iPhone plugged in (00008140)",
    "    0.6s  Cape’s iPhone · iOS 27.3 · Connected · Developer Mode off",
  ],
};

export const DEMO_MIRROR: MirrorStatus = {
  phase: "live",
  access: "allowed",
  deviceName: "Cape’s iPhone",
  width: 736,
  height: 1600,
  orientation: "portrait",
  seq: 1,
  error: null,
  events: [
    "    0.0s  mirror started",
    "    0.0s  camera access: Allowed",
    "    0.1s  asked macOS to list plugged-in iPhones as screens",
    "    1.1s  found Cape’s iPhone (iOS Device)",
    "    1.6s  first frame: 1206x2622 portrait, kept at 736x1600",
  ],
};

export const DEMO_MIRROR_OFF: MirrorStatus = {
  ...DEMO_MIRROR,
  phase: "noDevice",
  deviceName: null,
  width: null,
  height: null,
  orientation: null,
  events: DEMO_MIRROR.events.slice(0, 3),
};

export const DEMO_SETTINGS: SettingsView = {
  tokenSet: true,
  apiBase: "https://edart-api.lovable.app",
  apiBaseIsDefault: true,
  path: "/Users/cape/Library/Application Support/com.capemountco.edart/settings.json",
  instructions: "",
  allowTyping: false,
};

export const DEMO_AGENT_IDLE: AgentStatus = {
  running: false,
  stopping: false,
  step: 0,
  maxSteps: 25,
  entries: [],
};

const DEMO_STEPS: AgentStatus["entries"] = [
  { kind: "goal", text: "Open Settings and turn on Dark Mode" },
  {
    kind: "step",
    n: 1,
    thought: "Start from the Home Screen.",
    action: "Go Home",
  },
  {
    kind: "step",
    n: 2,
    thought: "Settings is in the second row.",
    action: "Tap (284, 512)",
  },
  {
    kind: "step",
    n: 3,
    thought: "Scroll to Display & Brightness.",
    action: "Swipe (370, 1200) → (370, 500)",
  },
  { kind: "step", n: 4, thought: "", action: "Tap (300, 880)" },
];

export const DEMO_AGENT_RUNNING: AgentStatus = {
  running: true,
  stopping: false,
  step: 4,
  maxSteps: 25,
  entries: DEMO_STEPS,
};

export const DEMO_AGENT_DONE: AgentStatus = {
  running: false,
  stopping: false,
  step: 5,
  maxSteps: 25,
  entries: [
    ...DEMO_STEPS,
    { kind: "step", n: 5, thought: "Choose Dark.", action: "Tap (520, 640)" },
    {
      kind: "finished",
      success: true,
      text: "Dark Mode is on. I opened Settings › Display & Brightness and chose Dark.",
    },
  ],
};

export function demoAgent(): AgentStatus {
  const chat = demoParam("chat");
  if (chat === "run") return DEMO_AGENT_RUNNING;
  if (chat === "done") return DEMO_AGENT_DONE;
  return DEMO_AGENT_IDLE;
}

/** The built-in playbook, shortened: the real one comes from Rust. */
const DEMO_PLAYBOOK = `You are trading on this iPhone in the Kalshi app only. Edart enforces hard limits and stops you if you break one, so follow them exactly:
- Use only the Kalshi app. Never open another app, a link, a notification or Settings.
- Trade only the "ZEC 15 min" up/down market.
- Every buy is exactly $0.10. On the order sheet tap the keypad keys 0 . 1 0, check that the amount reads $0.10, and only then slide "Slide to buy".
How to trade:
1. Open the ZEC 15 min market and read the target price, the current price, the time left and the Up and Down prices.
2. Consider buying a side only when its price is under 30% and the chart and recent momentum suggest it is likely to rise. If that is not clearly the case, wait.
3. After a buy, watch the position: wait up to 30 seconds at a time and check again.
4. When the position is up 50% to 100%, sell it to cash out. If it falls, let it expire.`;

const DEMO_TRADING_ON: TradingView = {
  enabled: true,
  stakeCents: 10,
  maxTradesPerDay: 10,
  maxLossCentsPerDay: 100,
  confirmEachBuy: false,
  maxSteps: 120,
  app: "Kalshi",
  market: "ZEC 15 min",
  worstCaseCents: 100,
  summary: "Kalshi only · $0.10 · 3/10 buys · $0.30/$1.00",
  playbook: DEMO_PLAYBOOK,
  playbookIsDefault: true,
  today: { day: "2026-11-19", trades: 3, spentCents: 30 },
  tradesLeft: 7,
  ledgerError: null,
  pending: null,
};

export function demoTrading(): TradingView {
  if (demoParam("trading") === "off") {
    return {
      ...DEMO_TRADING_ON,
      enabled: false,
      today: { day: "2026-11-19", trades: 0, spentCents: 0 },
      tradesLeft: 10,
      summary: "Kalshi only · $0.10 · 0/10 buys · $0.00/$1.00",
    };
  }
  if (demoParam("confirm") === "buy") {
    return {
      ...DEMO_TRADING_ON,
      confirmEachBuy: true,
      pending: {
        summary: "Up, $0.10, ZEC 15 min",
        buy: { side: "up", stakeCents: 10, market: "ZEC 15 min" },
        secondsLeft: 48,
      },
    };
  }
  return DEMO_TRADING_ON;
}

/** 2026-11-19, 8:14 pm local, in seconds. */
const DEMO_NOW = Date.UTC(2026, 10, 19, 20, 14) / 1000;

export const DEMO_LEDGER: LedgerEvent[] =
  demoParam("trading") === "off"
    ? []
    : [
        {
          time: DEMO_NOW,
          day: "2026-11-19",
          kind: "blocked",
          stakeCents: 0,
          reason: "The order amount is $1.00, and a buy must be exactly $0.10",
        },
        {
          time: DEMO_NOW - 9 * 60,
          day: "2026-11-19",
          kind: "buy",
          side: "down",
          stakeCents: 10,
          market: "ZEC 15 min",
        },
        {
          time: DEMO_NOW - 21 * 60,
          day: "2026-11-19",
          kind: "sell",
          stakeCents: 0,
          market: "ZEC 15 min",
        },
        {
          time: DEMO_NOW - 29 * 60,
          day: "2026-11-19",
          kind: "buy",
          side: "up",
          stakeCents: 10,
          market: "ZEC 15 min",
        },
        {
          time: DEMO_NOW - 47 * 60,
          day: "2026-11-19",
          kind: "blocked",
          stakeCents: 0,
          reason:
            "Left Kalshi: this screen isn’t Kalshi, so only Home is allowed",
        },
        {
          time: DEMO_NOW - 52 * 60,
          day: "2026-11-19",
          kind: "buy",
          side: "up",
          stakeCents: 10,
          market: "ZEC 15 min",
        },
      ];
