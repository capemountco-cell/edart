import { ChevronIcon, PlusIcon } from "../components/icons";
import { Details } from "../components/Details";
import { linkLabel, mirrorCopy, phaseCopy, usbPhaseCopy } from "../lib/phase";
import type { Edart } from "../lib/useEdart";
import type { Route } from "../App";
import { PauseButton, StatusPill } from "./Control";

/** The iPhone Edart can see and control, and how it is linked. */
export function Phones({
  app,
  go,
}: {
  app: Edart;
  go: (route: Route) => void;
}) {
  const links = linkLabel({
    mirror: app.mirrorHasPhone,
    usbControl: app.usbReady,
    bluetooth: app.bluetoothConnected,
  });
  const mirror = app.mirror;
  const size =
    mirror?.width && mirror.height
      ? `${mirror.width} × ${mirror.height}, ${mirror.orientation}`
      : null;
  const mac = app.bluetooth?.macName ?? "this Mac";

  return (
    <div className="page">
      <header className="page-head">
        <span className="eyebrow">Devices</span>
        <h1>
          Phones<span className="dim">.</span>
        </h1>
        <p className="lede">The iPhone Edart can see and control.</p>
      </header>

      <section aria-labelledby="connected-title" className="group">
        <div className="group-head">
          <h2 id="connected-title" className="eyebrow">
            {app.connected ? "Connected" : "No iPhone yet"}
          </h2>
          {app.connected && <span className="fine">1 of 1</span>}
        </div>
        {app.connected ? (
          <button
            type="button"
            className="card row-link"
            onClick={() => go("control")}
          >
            <span className="phone-glyph" aria-hidden="true" />
            <span className="row-text">
              <strong>{app.phoneName}</strong>
              <span>
                {[app.usb?.iosVersion && `iOS ${app.usb.iosVersion}`, links]
                  .filter(Boolean)
                  .join(" · ")}
              </span>
            </span>
            <StatusPill app={app} />
            <ChevronIcon />
          </button>
        ) : (
          <button
            type="button"
            className="card add-row"
            onClick={() => go("setup")}
          >
            <PlusIcon />
            Connect an iPhone
          </button>
        )}
        {(app.bluetooth?.paused || app.bluetoothConnected) && (
          <div className="button-row">
            <PauseButton app={app} />
            <span className="fine">
              {app.bluetooth?.paused
                ? "Paused: your iPhone is free and still paired."
                : "Pause lets go of your iPhone for a while without unpairing."}
            </span>
          </div>
        )}
      </section>

      <section aria-labelledby="links-title" className="group">
        <h2 id="links-title" className="eyebrow">
          How it’s linked
        </h2>
        <div className="card list">
          <div className="list-row">
            <span className="row-text">
              <strong>Screen</strong>
              <span>
                {mirrorCopy(mirror?.phase ?? "off").title}
                {size && ` · ${size}`}
              </span>
            </span>
            <span className={`state ${app.mirrorLive ? "good" : ""}`}>
              {app.mirrorLive ? "USB" : "Off"}
            </span>
          </div>
          <div className="list-row">
            <span className="row-text">
              <strong>Touch and keyboard</strong>
              <span>
                {app.usbReady
                  ? usbPhaseCopy("ready", app.phoneName).detail
                  : phaseCopy(app.bluetooth?.phase ?? "unsupported", mac).title}
              </span>
            </span>
            <span className={`state ${app.controlReady ? "good" : ""}`}>
              {app.usbReady
                ? "USB"
                : app.bluetoothConnected
                  ? "Bluetooth"
                  : "Off"}
            </span>
          </div>
        </div>
        {!app.connected || !app.mirrorLive || !app.controlReady ? (
          <button
            type="button"
            className="secondary"
            onClick={() => go("setup")}
          >
            Open Permissions
          </button>
        ) : null}
      </section>

      <Details app={app} open />
    </div>
  );
}
