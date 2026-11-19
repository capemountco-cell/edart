/**
 * Sample data for `?demo`: every screen renders in a plain browser tab,
 * for screenshots and design work. Nothing here runs in the app.
 *
 * `?demo&screen=phones` picks a screen; `&chat=run` or `&chat=done` fills
 * the agent chat; `&mirror=off` shows the Control screen without a
 * mirror; `&sheet=agent` opens the agent sheet in a narrow window.
 */
import type {
  AgentStatus,
  BluetoothStatus,
  MirrorStatus,
  SettingsView,
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
