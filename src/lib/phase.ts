import type { MirrorPhase, Phase, UsbPhase } from "./native";

/**
 * Headline copy for each phase. The phase itself is decided in Rust.
 * `mac` is the Mac's computer name: the iPhone lists Edart under it.
 */
export function phaseCopy(
  phase: Phase,
  mac: string,
): { title: string; detail: string } {
  switch (phase) {
    case "unsupported":
      return {
        title: "Needs a Mac",
        detail: "Bluetooth control only works in the Mac app.",
      };
    case "notStarted":
      return {
        title: "Bluetooth is off in Edart",
        detail:
          "Turn it on so your iPhone can find this Mac. macOS will ask for permission once.",
      };
    case "needsPermission":
      return {
        title: "Allow Bluetooth",
        detail:
          "Open System Settings › Privacy & Security › Bluetooth and switch Edart on.",
      };
    case "bluetoothOff":
      return {
        title: "Bluetooth is off",
        detail: "Turn on Bluetooth in Control Center on this Mac.",
      };
    case "starting":
      return {
        title: "Starting…",
        detail: "Setting up this Mac as a Bluetooth mouse and keyboard.",
      };
    case "broken":
      return {
        title: "macOS refused the mouse service",
        detail:
          "Your iPhone can’t use Edart as a mouse. Quit and reopen Edart, then try again.",
      };
    case "waitingForPhone":
      return {
        title: "Ready to pair",
        detail: `On your iPhone, tap “${mac}” in AssistiveTouch › Devices › Bluetooth Devices.`,
      };
    case "pairing":
      return {
        title: "Almost there: pairing didn’t finish",
        detail:
          "Your iPhone found Edart, but the secure pairing didn’t complete. This usually means old pairing leftovers. Forget the device on both sides: on this Mac in System Settings › Bluetooth (leave that window open), and on the iPhone in AssistiveTouch › Devices › Bluetooth Devices › ⓘ › Forget This Device. Turn the iPhone’s Bluetooth off and on, press Restart Bluetooth here, then tap the Mac’s name on the iPhone and accept Pair on both.",
      };
    case "connected":
      return {
        title: "iPhone connected",
        detail: "Edart can tap, swipe and type on your iPhone.",
      };
    case "paused":
      return {
        title: "Paused",
        detail:
          "Edart has let go of your iPhone: no pointer, no taps. The pairing is kept. Press Resume to take control again.",
      };
  }
}

/** Headline copy for each step of the USB setup. The phase is decided in Rust. */
export function usbPhaseCopy(
  phase: UsbPhase,
  phone: string,
): { title: string; detail: string } {
  switch (phase) {
    case "unsupported":
      return {
        title: "Needs a Mac",
        detail: "USB control only works in the Mac app.",
      };
    case "hostMissing":
      return {
        title: "This Mac can’t talk to iPhones",
        detail:
          "macOS’s device framework didn’t load. Update macOS, then reopen Edart.",
      };
    case "noDevice":
      return {
        title: "Plug in your iPhone",
        detail: "Connect it to this Mac with a USB cable and unlock it.",
      };
    case "restarting":
      return {
        title: "Your iPhone is restarting",
        detail:
          "Keep the cable in. When it’s back, unlock it and tap Turn On when it asks about Developer Mode.",
      };
    case "untrusted":
      return {
        title: "Tap Trust on your iPhone",
        detail:
          "Unlock the iPhone, tap Trust and enter its passcode. No prompt? Unplug the cable and plug it in again.",
      };
    case "locked":
      return {
        title: "Unlock your iPhone",
        detail: "It has been locked since it was plugged in.",
      };
    case "oldIos":
      return {
        title: "This iPhone needs iOS 17 or later",
        detail:
          "USB control uses services that older versions of iOS don’t have. Bluetooth below may still work.",
      };
    case "developerModeOff":
      return {
        title: "Turn on Developer Mode",
        detail: `${phone} only accepts touches from a Mac in Developer Mode. Edart can switch it on; the iPhone restarts once.`,
      };
    case "developerModeManual":
      return {
        title: "Turn on Developer Mode in Settings",
        detail:
          "On the iPhone, open Settings › Privacy & Security › Developer Mode, switch it on and tap Restart. After the restart, tap Turn On and enter the passcode.",
      };
    case "confirmDeveloperMode":
      return {
        title: "Confirm on your iPhone",
        detail:
          "Unlock the iPhone, tap Turn On when it asks about Developer Mode, and enter the passcode.",
      };
    case "xcodeMissing":
      return {
        title: "Install Xcode",
        detail:
          "Edart reaches the iPhone through Xcode’s device service. Install Xcode from the App Store, open it once, then come back.",
      };
    case "notPrepared":
      return {
        title: "Almost there",
        detail:
          "Connect to the iPhone’s touch and keyboard services. Keep the iPhone unlocked. The first time can take a minute.",
      };
    case "preparing":
      return {
        title: "Connecting…",
        detail: "Keep the iPhone unlocked. The first time can take a minute.",
      };
    case "diskImageMissing":
      return {
        title: "The iPhone’s developer tools aren’t loaded",
        detail:
          "Open Xcode, choose Window › Devices and Simulators, select the iPhone and wait until it’s ready. Then connect again.",
      };
    case "serviceUnavailable":
      return {
        title: "Couldn’t reach the iPhone’s touch service",
        detail: "Unlock the iPhone and connect again.",
      };
    case "ready":
      return {
        title: "iPhone connected",
        detail: `Edart can tap, swipe and type on ${phone} over USB.`,
      };
  }
}

export type StepState = "done" | "current" | "todo";

/**
 * Where each of the four setup steps stands: trust, Developer Mode,
 * restart and confirm, connect.
 */
export function usbStepStates(
  phase: UsbPhase,
): [StepState, StepState, StepState, StepState] {
  const upTo = (current: number): StepState[] =>
    [0, 1, 2, 3].map((i) =>
      i < current ? "done" : i === current ? "current" : "todo",
    );
  const at = (current: number) =>
    upTo(current) as [StepState, StepState, StepState, StepState];
  switch (phase) {
    case "unsupported":
    case "hostMissing":
    case "noDevice":
    case "untrusted":
    case "locked":
    case "oldIos":
      return at(0);
    case "developerModeOff":
      return at(1);
    case "developerModeManual":
    case "restarting":
    case "confirmDeveloperMode":
      return at(2);
    case "xcodeMissing":
    case "notPrepared":
    case "preparing":
    case "diskImageMissing":
    case "serviceUnavailable":
      return at(3);
    case "ready":
      return at(4);
  }
}

/** Headline copy for the screen mirror. The phase is decided in Rust. */
export function mirrorCopy(phase: MirrorPhase): {
  title: string;
  detail: string;
} {
  switch (phase) {
    case "unsupported":
      return {
        title: "Needs a Mac",
        detail: "The iPhone’s screen only shows in the Mac app.",
      };
    case "off":
      return {
        title: "See your iPhone’s screen",
        detail:
          "Plug the iPhone in with a USB cable. macOS lists it as a camera, so it asks for Camera access once.",
      };
    case "needsPermission":
      return {
        title: "Allow Camera access",
        detail:
          "macOS is asking. It lists an iPhone on a cable as a camera; Edart doesn’t use this Mac’s own camera.",
      };
    case "denied":
      return {
        title: "Camera access is off",
        detail:
          "Open System Settings › Privacy & Security › Camera and switch Edart on, then reopen Edart.",
      };
    case "noDevice":
      return {
        title: "Plug in your iPhone",
        detail:
          "Use a USB cable, unlock the iPhone and tap Trust. The screen appears a few seconds later.",
      };
    case "starting":
      return {
        title: "Starting the screen…",
        detail: "Keep the iPhone unlocked.",
      };
    case "stalled":
      return {
        title: "The iPhone isn’t sending its screen",
        detail:
          "Unlock it, then unplug the cable and plug it in again. If another app is showing this iPhone’s screen, quit it.",
      };
    case "dark":
      return {
        title: "The iPhone’s screen is off",
        detail: "Wake the iPhone and unlock it.",
      };
    case "live":
      return {
        title: "Screen is live",
        detail: "Click and drag on it like a finger.",
      };
  }
}

/** How the phone is linked, for the Phones screen: "USB + Bluetooth". */
export function linkLabel(links: {
  mirror: boolean;
  usbControl: boolean;
  bluetooth: boolean;
}): string {
  const parts = [];
  if (links.mirror || links.usbControl) parts.push("USB");
  if (links.bluetooth) parts.push("Bluetooth");
  return parts.length > 0 ? parts.join(" + ") : "Not connected";
}
