import { useCallback, useEffect, useRef, useState } from "react";

import {
  DEMO_AGENT_DONE,
  DEMO_BLUETOOTH,
  DEMO_LEDGER,
  DEMO_MIRROR,
  DEMO_MIRROR_OFF,
  DEMO_SETTINGS,
  DEMO_USB,
  demoAgent,
  demoParam,
  demoTrading,
  isDemo,
} from "./demo";
import {
  agentStatus,
  bluetoothStatus,
  clearAgent,
  confirmTrade,
  getSettings,
  getTrading,
  mirrorStatus,
  nativeAvailable,
  onAgentChanged,
  onTradingChanged,
  perform,
  setTrading,
  startAgent,
  stopAgent,
  tradingLedger,
  usbStatus,
  type AgentStatus,
  type BluetoothStatus,
  type Gesture,
  type LedgerEvent,
  type MirrorStatus,
  type Screen,
  type SettingsView,
  type TradingChange,
  type TradingView,
  type UsbStatus,
} from "./native";

/** Screen units for gestures when there is no mirror: iPhone points. */
const FALLBACK_SCREEN: Screen = { width: 390, height: 844 };

export const errorText = (cause: unknown) =>
  cause instanceof Error ? cause.message : String(cause);

/** Why the agent can't run right now, and where to fix it. */
export interface AgentBlocker {
  text: string;
  fix: { label: string; route: "preferences" | "setup" };
}

/**
 * Everything the screens show, read from the native side on a timer, plus
 * the few actions more than one screen needs.
 */
export function useEdart() {
  const demo = isDemo();
  const [bluetooth, setBluetooth] = useState<BluetoothStatus | null>(
    demo ? DEMO_BLUETOOTH : null,
  );
  const [usb, setUsb] = useState<UsbStatus | null>(demo ? DEMO_USB : null);
  const [mirror, setMirror] = useState<MirrorStatus | null>(
    demo
      ? demoParam("mirror") === "off"
        ? DEMO_MIRROR_OFF
        : DEMO_MIRROR
      : null,
  );
  const [agent, setAgent] = useState<AgentStatus>(demoAgent);
  const [settings, setSettings] = useState<SettingsView | null>(
    demo ? DEMO_SETTINGS : null,
  );
  const [trading, setTradingView] = useState<TradingView | null>(
    demo ? demoTrading : null,
  );
  const [ledger, setLedger] = useState<LedgerEvent[]>(demo ? DEMO_LEDGER : []);
  /** True once the first statuses have arrived (or never will). */
  const [loaded, setLoaded] = useState(demo || !nativeAvailable());
  const [loadError, setLoadError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (demo || !nativeAvailable()) return;
    try {
      const [bt, cable, screen] = await Promise.all([
        bluetoothStatus(),
        usbStatus(),
        mirrorStatus(),
      ]);
      setBluetooth(bt);
      setUsb(cable);
      setMirror(screen);
      setLoadError(null);
    } catch (cause) {
      setLoadError(errorText(cause));
    } finally {
      setLoaded(true);
    }
  }, [demo]);

  const refreshAgent = useCallback(async () => {
    if (demo || !nativeAvailable()) return;
    try {
      setAgent(await agentStatus());
    } catch {
      // The next tick tries again.
    }
  }, [demo]);

  const refreshSettings = useCallback(async () => {
    if (demo || !nativeAvailable()) return;
    try {
      setSettings(await getSettings());
    } catch {
      // Preferences shows the error when the user saves.
    }
  }, [demo]);

  const refreshTrading = useCallback(async () => {
    if (demo || !nativeAvailable()) return;
    try {
      const view = await getTrading();
      setTradingView(view);
      setLedger(await tradingLedger());
    } catch {
      // The Trading screen shows the error when the user changes a rule.
    }
  }, [demo]);

  useEffect(() => {
    void refresh();
    void refreshAgent();
    void refreshSettings();
    void refreshTrading();
    const timer = window.setInterval(() => {
      void refresh();
      void refreshAgent();
      // The confirmation's countdown and the day's count move on their own.
      void refreshTrading();
    }, 1000);
    let stopListening = () => {};
    let gone = false;
    void onAgentChanged(() => void refreshAgent()).then((stop) => {
      if (gone) stop();
      else stopListening = stop;
    });
    let stopTrading = () => {};
    void onTradingChanged(() => void refreshTrading()).then((stop) => {
      if (gone) stop();
      else stopTrading = stop;
    });
    return () => {
      gone = true;
      window.clearInterval(timer);
      stopListening();
      stopTrading();
    };
  }, [refresh, refreshAgent, refreshSettings, refreshTrading]);

  const bluetoothConnected = bluetooth?.phase === "connected";
  const usbReady = usb?.phase === "ready";
  const controlReady = bluetoothConnected || usbReady;
  const mirrorLive = mirror?.phase === "live";
  const mirrorHasPhone =
    mirror !== null &&
    ["starting", "stalled", "dark", "live"].includes(mirror.phase);
  const screen: Screen =
    mirror?.width && mirror.height
      ? { width: mirror.width, height: mirror.height }
      : FALLBACK_SCREEN;
  const phoneName = usb?.deviceName ?? mirror?.deviceName ?? "iPhone";
  const connected = controlReady || mirrorHasPhone;

  const agentBlocker: AgentBlocker | null = !settings?.tokenSet
    ? {
        text: "Add your AI token to use the agent.",
        fix: { label: "Open Preferences", route: "preferences" },
      }
    : !controlReady
      ? {
          text: "Pair the iPhone over Bluetooth so the agent can tap and type.",
          fix: { label: "Open Permissions", route: "setup" },
        }
      : !mirrorLive
        ? {
            text: "The agent needs to see the screen. Plug the iPhone in with a USB cable, unlock it and allow Camera access.",
            fix: { label: "Open Permissions", route: "setup" },
          }
        : null;

  /** Send one gesture to the phone. Rejects with a sentence. */
  const { width, height } = screen;
  const send = useCallback(
    async (gesture: Gesture) => {
      if (demo) return;
      await perform(gesture, { width, height });
    },
    [demo, width, height],
  );

  // In a browser tab the agent is acted out, so the chat can be seen moving.
  const demoTimer = useRef<number | null>(null);
  useEffect(
    () => () => {
      if (demoTimer.current !== null) window.clearInterval(demoTimer.current);
    },
    [],
  );
  const stopDemo = () => {
    if (demoTimer.current !== null) window.clearInterval(demoTimer.current);
    demoTimer.current = null;
  };

  const ask = useCallback(
    async (goal: string) => {
      if (!demo) {
        await startAgent(goal);
        await refreshAgent();
        return;
      }
      stopDemo();
      const script = DEMO_AGENT_DONE.entries.slice(1);
      let shown = 0;
      setAgent((a) => ({
        ...a,
        running: true,
        stopping: false,
        step: 0,
        entries: [...a.entries, { kind: "goal", text: goal }],
      }));
      demoTimer.current = window.setInterval(() => {
        const entry = script[shown++];
        const last = entry === undefined || shown >= script.length;
        if (last) stopDemo();
        setAgent((a) => ({
          ...a,
          running: !last,
          step: entry?.kind === "step" ? entry.n : a.step,
          entries: entry ? [...a.entries, entry] : a.entries,
        }));
      }, 900);
    },
    [demo, refreshAgent],
  );

  const stop = useCallback(async () => {
    if (!demo) {
      await stopAgent();
      await refreshAgent();
      return;
    }
    stopDemo();
    setAgent((a) => ({
      ...a,
      running: false,
      entries: [
        ...a.entries,
        { kind: "finished", success: false, text: "Stopped." },
      ],
    }));
  }, [demo, refreshAgent]);

  const newChat = useCallback(async () => {
    if (!demo) {
      await clearAgent();
      await refreshAgent();
      return;
    }
    stopDemo();
    setAgent((a) => ({ ...a, running: false, step: 0, entries: [] }));
  }, [demo, refreshAgent]);

  /** Change a trading rule. Rejects with a sentence when out of range. */
  const changeTrading = useCallback(
    async (change: TradingChange) => {
      if (!demo) {
        setTradingView(await setTrading(change));
        return;
      }
      setTradingView((view) => {
        if (!view) return view;
        const next = { ...view, ...change };
        const playbook = change.playbook ?? view.playbook;
        return {
          ...next,
          playbook: playbook === "" ? view.playbook : playbook,
          worstCaseCents: Math.min(
            next.maxLossCentsPerDay,
            next.stakeCents * next.maxTradesPerDay,
          ),
        };
      });
    },
    [demo],
  );

  /** Start the agent on the trading playbook. */
  const startTradingRun = useCallback(
    () => ask(demo ? "Trading run: follow the playbook" : ""),
    [ask, demo],
  );

  /** Answer the buy that is waiting for approval. */
  const answerBuy = useCallback(
    async (approve: boolean) => {
      if (!demo) {
        await confirmTrade(approve);
        await refreshTrading();
        return;
      }
      setTradingView((view) => (view ? { ...view, pending: null } : view));
    },
    [demo, refreshTrading],
  );

  return {
    demo,
    loaded,
    loadError,
    bluetooth,
    usb,
    mirror,
    agent,
    settings,
    setSettings,
    trading,
    ledger,
    changeTrading,
    answerBuy,
    startTradingRun,
    refresh,
    bluetoothConnected,
    usbReady,
    controlReady,
    mirrorLive,
    mirrorHasPhone,
    connected,
    screen,
    phoneName,
    agentBlocker,
    send,
    ask,
    stop,
    newChat,
  };
}

export type Edart = ReturnType<typeof useEdart>;

/** True while the window is at most `width` pixels wide. */
export function useNarrow(width: number): boolean {
  const query = `(max-width: ${width}px)`;
  const [narrow, setNarrow] = useState(
    () => typeof window !== "undefined" && window.matchMedia(query).matches,
  );
  useEffect(() => {
    const media = window.matchMedia(query);
    const update = () => setNarrow(media.matches);
    update();
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, [query]);
  return narrow;
}
