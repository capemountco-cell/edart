import { useEffect, useState } from "react";

import { Connectors as ConnectorCard } from "../components/Connectors";
import { isDemo } from "../lib/demo";
import { DEMO_MCP_STATUS, mcpStatus, type McpStatus } from "../lib/mcp";
import { nativeAvailable } from "../lib/native";

/** Use the iPhone from other AI tools, through Edart's local MCP server. */
export function Connectors() {
  const demo = isDemo();
  const [status, setStatus] = useState<McpStatus | null>(
    demo ? DEMO_MCP_STATUS : null,
  );

  useEffect(() => {
    if (demo || !nativeAvailable()) return;
    let live = true;
    const refresh = () =>
      mcpStatus()
        .then((s) => live && setStatus(s))
        .catch(() => {});
    void refresh();
    const timer = window.setInterval(refresh, 2000);
    return () => {
      live = false;
      window.clearInterval(timer);
    };
  }, [demo]);

  return (
    <div className="page">
      <header className="page-head">
        <span className="eyebrow">Connectors</span>
        <h1>
          Connectors<span className="dim">.</span>
        </h1>
        <p className="lede">
          Use your iPhone from the AI tools you already have.
        </p>
      </header>
      <ConnectorCard status={status} onChange={setStatus} />
    </div>
  );
}
