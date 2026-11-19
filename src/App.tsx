import { useEffect, useState, type ReactNode } from "react";

import {
  CloseIcon,
  ConnectorsIcon,
  MenuIcon,
  PhoneIcon,
  PhonesIcon,
  ShieldIcon,
  SlidersIcon,
  TrendIcon,
} from "./components/icons";
import { ConfirmBuy } from "./components/ConfirmBuy";
import { demoParam } from "./lib/demo";
import { nativeAvailable } from "./lib/native";
import { useEdart, useNarrow } from "./lib/useEdart";
import { Connectors } from "./screens/Connectors";
import { Control, StatusPill } from "./screens/Control";
import { Phones } from "./screens/Phones";
import { Preferences } from "./screens/Preferences";
import { Setup } from "./screens/Setup";
import { Trading } from "./screens/Trading";

export type Route =
  | "control"
  | "trading"
  | "phones"
  | "setup"
  | "connectors"
  | "preferences";

const NAV: { route: Route; label: string; icon: ReactNode }[] = [
  { route: "control", label: "Control", icon: <PhoneIcon /> },
  { route: "trading", label: "Trading", icon: <TrendIcon /> },
  { route: "phones", label: "Phones", icon: <PhonesIcon /> },
  { route: "setup", label: "Permissions", icon: <ShieldIcon /> },
  { route: "connectors", label: "Connectors", icon: <ConnectorsIcon /> },
  { route: "preferences", label: "Preferences", icon: <SlidersIcon /> },
];

const isRoute = (value: string | null): value is Route =>
  NAV.some((item) => item.route === value);

/** At or below this width the window gets the compact layout. */
const NARROW_PX = 700;

export function App() {
  const app = useEdart();
  const narrow = useNarrow(NARROW_PX);
  const [menu, setMenu] = useState(false);
  // The first screen is Control when a phone is connected, else Setup.
  // It is chosen once, when the first statuses arrive, and then left to
  // the user.
  const [route, setRoute] = useState<Route | null>(() => {
    const asked = demoParam("screen");
    return app.demo ? (isRoute(asked) ? asked : "control") : null;
  });
  useEffect(() => {
    if (route === null && app.loaded) {
      setRoute(app.connected ? "control" : "setup");
    }
  }, [route, app.loaded, app.connected]);

  const go = (next: Route) => {
    setRoute(next);
    setMenu(false);
  };

  const links = (
    <ul className="nav-list">
      {NAV.map((item) => (
        <li key={item.route}>
          <button
            type="button"
            className="nav-link"
            aria-current={route === item.route ? "page" : undefined}
            title={item.label}
            onClick={() => go(item.route)}
          >
            {item.icon}
            <span>{item.label}</span>
          </button>
        </li>
      ))}
    </ul>
  );

  return (
    <div className={`app ${narrow ? "narrow" : ""}`}>
      {narrow ? (
        <header className="topbar">
          <span className="wordmark small">EDART</span>
          <div className="topbar-right">
            <StatusPill app={app} name />
            <button
              type="button"
              className="icon-button"
              aria-label="Menu"
              aria-expanded={menu}
              onClick={() => setMenu(true)}
            >
              <MenuIcon />
            </button>
          </div>
        </header>
      ) : (
        <nav className="sidebar" aria-label="Main">
          <span className="wordmark">EDART</span>
          {links}
          <div className="sidebar-note">
            <span className="eyebrow">Local · No account</span>
            <span>
              Runs on this Mac. Screenshots leave it only when you ask the
              agent.
            </span>
          </div>
        </nav>
      )}

      <main className="main">
        {!nativeAvailable() && !app.demo && (
          <p className="banner" role="status">
            This is the app’s page in a browser, so nothing is connected. Open
            the Edart app, or add ?demo to the address to see sample data.
          </p>
        )}
        {app.loadError && (
          <p className="banner error" role="alert">
            {app.loadError}
          </p>
        )}
        {route === "control" && <Control app={app} narrow={narrow} go={go} />}
        {route === "trading" && <Trading app={app} go={go} />}
        {route === "phones" && <Phones app={app} go={go} />}
        {route === "setup" && <Setup app={app} go={go} />}
        {route === "connectors" && <Connectors />}
        {route === "preferences" && <Preferences app={app} />}
      </main>

      {app.trading?.pending && (
        <ConfirmBuy
          pending={app.trading.pending}
          onAnswer={(approve) => void app.answerBuy(approve)}
        />
      )}

      {narrow && menu && (
        <div className="sheet-layer">
          <button
            type="button"
            className="sheet-scrim"
            aria-label="Close menu"
            onClick={() => setMenu(false)}
          />
          <nav className="sheet menu-sheet" aria-label="Main">
            <div className="sheet-head">
              <span className="eyebrow">Menu</span>
              <button
                type="button"
                className="icon-button"
                aria-label="Close menu"
                onClick={() => setMenu(false)}
              >
                <CloseIcon />
              </button>
            </div>
            {links}
          </nav>
        </div>
      )}
    </div>
  );
}
