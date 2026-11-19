import { useState, type FormEvent } from "react";

import { CheckIcon } from "../components/icons";
import {
  setSettings,
  testSettings,
  type ConnectionTest,
  type SettingsChange,
} from "../lib/native";
import { errorText, type Edart } from "../lib/useEdart";

/** The AI token and, for testing, the address of the AI service. */
export function Preferences({ app }: { app: Edart }) {
  const settings = app.settings;
  const [token, setToken] = useState("");
  const [address, setAddress] = useState<string | null>(null);
  const [busy, setBusy] = useState<"token" | "address" | "test" | null>(null);
  const [saved, setSaved] = useState<"token" | "address" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [test, setTest] = useState<ConnectionTest | null>(null);

  const save = async (what: "token" | "address", change: SettingsChange) => {
    setBusy(what);
    setError(null);
    setSaved(null);
    setTest(null);
    try {
      if (!app.demo) app.setSettings(await setSettings(change));
      else if (settings) {
        app.setSettings({
          ...settings,
          tokenSet:
            change.token === undefined
              ? settings.tokenSet
              : change.token !== "",
        });
      }
      setSaved(what);
      if (what === "token") setToken("");
      else setAddress(null);
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setBusy(null);
    }
  };

  const runTest = async () => {
    setBusy("test");
    setError(null);
    setTest(null);
    try {
      setTest(
        app.demo
          ? {
              reachable: true,
              token: "accepted",
              message: "Connected. The token works.",
            }
          : await testSettings(),
      );
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setBusy(null);
    }
  };

  const submitToken = (event: FormEvent) => {
    event.preventDefault();
    if (token.trim() !== "") void save("token", { token });
  };
  const submitAddress = (event: FormEvent) => {
    event.preventDefault();
    if (address !== null) void save("address", { apiBase: address });
  };

  const tokenSet = settings?.tokenSet ?? false;
  const testGood = test?.reachable && test.token === "accepted";

  return (
    <div className="page">
      <header className="page-head">
        <span className="eyebrow">Preferences</span>
        <h1>
          Preferences<span className="dim">.</span>
        </h1>
        <p className="lede">
          The agent thinks with an AI service. It needs your token to use it.
        </p>
      </header>

      <section className="group" aria-labelledby="ai-title">
        <h2 id="ai-title" className="eyebrow">
          AI service
        </h2>
        <div className="card list">
          <form className="list-row wrap" onSubmit={submitToken}>
            <div className="row-text">
              <label htmlFor="token">
                <strong>AI token</strong>
              </label>
              <span>
                {tokenSet
                  ? "A token is saved. Paste a new one to replace it."
                  : "Paste the token you chose for the Edart AI service."}
              </span>
            </div>
            <div className="row-controls">
              <input
                id="token"
                className="field"
                type="password"
                autoComplete="off"
                spellCheck={false}
                placeholder={
                  tokenSet ? "•••••••••••• saved" : "Paste your token"
                }
                value={token}
                onChange={(e) => {
                  setToken(e.target.value);
                  setSaved(null);
                }}
              />
              <button
                type="submit"
                className="primary small"
                disabled={busy !== null || token.trim() === ""}
              >
                {busy === "token" ? "Saving…" : "Save"}
              </button>
            </div>
            {(saved === "token" || (tokenSet && token === "")) && (
              <p className="row-note good">
                <CheckIcon size={16} />
                {saved === "token" ? "Saved" : "Token saved"}
              </p>
            )}
          </form>

          <div className="list-row wrap">
            <div className="row-text">
              <strong>Check the connection</strong>
              <span>
                Asks the service whether it is up and whether it accepts the
                saved token. Nothing from your iPhone is sent.
              </span>
            </div>
            <div className="row-controls end">
              {tokenSet && (
                <button
                  type="button"
                  className="secondary small"
                  disabled={busy !== null}
                  onClick={() => void save("token", { token: "" })}
                >
                  Remove token
                </button>
              )}
              <button
                type="button"
                className="secondary small"
                disabled={busy !== null}
                onClick={() => void runTest()}
              >
                {busy === "test" ? "Testing…" : "Test"}
              </button>
            </div>
            {test && (
              <p
                className={`row-note ${testGood ? "good" : "bad"}`}
                role="status"
              >
                {testGood && <CheckIcon size={16} />}
                {test.message}
              </p>
            )}
          </div>
        </div>
        {error && (
          <p className="fine error" role="alert">
            {error}
          </p>
        )}
      </section>

      <section className="group" aria-labelledby="advanced-title">
        <h2 id="advanced-title" className="eyebrow">
          Advanced
        </h2>
        <div className="card list">
          <form className="list-row wrap" onSubmit={submitAddress}>
            <div className="row-text">
              <label htmlFor="address">
                <strong>Service address</strong>
              </label>
              <span>
                Change this only to test against another server. It must start
                with https://.
              </span>
            </div>
            <div className="row-controls">
              <input
                id="address"
                className="field mono"
                type="url"
                autoComplete="off"
                spellCheck={false}
                value={address ?? settings?.apiBase ?? ""}
                onChange={(e) => {
                  setAddress(e.target.value);
                  setSaved(null);
                }}
              />
              <button
                type="submit"
                className="secondary small"
                disabled={busy !== null || address === null}
              >
                {busy === "address" ? "Saving…" : "Save"}
              </button>
              {settings && !settings.apiBaseIsDefault && (
                <button
                  type="button"
                  className="secondary small"
                  disabled={busy !== null}
                  onClick={() => void save("address", { apiBase: "" })}
                >
                  Reset
                </button>
              )}
            </div>
            {saved === "address" && (
              <p className="row-note good">
                <CheckIcon size={16} />
                Saved
              </p>
            )}
          </form>
        </div>
      </section>

      <section className="group" aria-labelledby="privacy-title">
        <h2 id="privacy-title" className="eyebrow">
          What leaves this Mac
        </h2>
        <div className="card prose">
          <p>
            While the agent works, each step sends your request, a screenshot of
            the iPhone and the steps so far to the service address above.
            Controlling the iPhone yourself sends nothing anywhere.
          </p>
          <p>
            The token is kept in a file only your user account on this Mac can
            read, and is sent only to that address:
          </p>
          {settings && <code className="path">{settings.path}</code>}
        </div>
      </section>
    </div>
  );
}
