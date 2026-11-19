const TOOLS = ["screenshot", "tap", "swipe", "type", "home"];

/** Where other AI tools will plug in. Not built yet, and says so. */
export function Connectors() {
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

      <section className="card feature" aria-labelledby="mcp-title">
        <span className="pill pill-idle">Coming soon</span>
        <div>
          <h2 id="mcp-title">MCP server</h2>
          <p>
            Edart will offer your iPhone to any MCP client, such as a coding
            agent or a desktop assistant, through a server that only answers on
            this Mac. Until then, the agent on the Control screen is the way to
            hand your iPhone a task.
          </p>
        </div>
        <div className="tags" aria-label="Planned tools">
          {TOOLS.map((tool) => (
            <span key={tool} className="tag">
              {tool}
            </span>
          ))}
        </div>
      </section>
    </div>
  );
}
