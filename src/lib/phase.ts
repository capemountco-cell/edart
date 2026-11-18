import type { Phase } from "./native";

/** Headline copy for each phase. The phase itself is decided in Rust. */
export const phaseCopy: Record<Phase, { title: string; detail: string }> = {
  unsupported: {
    title: "Needs a Mac",
    detail: "Bluetooth control only works in the Mac app.",
  },
  notStarted: {
    title: "Bluetooth is off in Edart",
    detail: "Turn it on so your iPhone can find Edart. macOS will ask for permission once.",
  },
  needsPermission: {
    title: "Allow Bluetooth",
    detail: "Open System Settings › Privacy & Security › Bluetooth and switch Edart on.",
  },
  bluetoothOff: {
    title: "Bluetooth is off",
    detail: "Turn on Bluetooth in Control Center on this Mac.",
  },
  starting: {
    title: "Starting…",
    detail: "Setting up Edart as a Bluetooth mouse and keyboard.",
  },
  broken: {
    title: "macOS refused the mouse service",
    detail: "Your iPhone can’t use Edart as a mouse. Quit and reopen Edart, then try again.",
  },
  waitingForPhone: {
    title: "Ready to pair",
    detail: "On your iPhone, pick “Edart” in AssistiveTouch › Devices › Bluetooth Devices.",
  },
  connected: {
    title: "iPhone connected",
    detail: "Edart can tap, swipe and type on your iPhone.",
  },
};
