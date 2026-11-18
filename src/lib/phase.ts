import type { Phase } from "./native";

/**
 * Headline copy for each phase. The phase itself is decided in Rust.
 * `mac` is the Mac's computer name: the iPhone lists Edart under it.
 */
export function phaseCopy(phase: Phase, mac: string): { title: string; detail: string } {
  switch (phase) {
    case "unsupported":
      return { title: "Needs a Mac", detail: "Bluetooth control only works in the Mac app." };
    case "notStarted":
      return {
        title: "Bluetooth is off in Edart",
        detail: "Turn it on so your iPhone can find this Mac. macOS will ask for permission once.",
      };
    case "needsPermission":
      return {
        title: "Allow Bluetooth",
        detail: "Open System Settings › Privacy & Security › Bluetooth and switch Edart on.",
      };
    case "bluetoothOff":
      return { title: "Bluetooth is off", detail: "Turn on Bluetooth in Control Center on this Mac." };
    case "starting":
      return { title: "Starting…", detail: "Setting up this Mac as a Bluetooth mouse and keyboard." };
    case "broken":
      return {
        title: "macOS refused the mouse service",
        detail: "Your iPhone can’t use Edart as a mouse. Quit and reopen Edart, then try again.",
      };
    case "waitingForPhone":
      return {
        title: "Ready to pair",
        detail: `On your iPhone, tap “${mac}” in AssistiveTouch › Devices › Bluetooth Devices.`,
      };
    case "pairing":
      return {
        title: "Almost there",
        detail: "Your iPhone found Edart. Tap Pair if it asks. Still spinning? Tap ⓘ › Forget This Device, then pick the Mac again.",
      };
    case "connected":
      return { title: "iPhone connected", detail: "Edart can tap, swipe and type on your iPhone." };
  }
}
