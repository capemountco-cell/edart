import { describe, expect, it } from "vitest";

import {
  DEMO_MCP_STATUS,
  mcpCallTime,
  mcpHeadline,
  mcpSetEnabled,
  mcpSetups,
  mcpStatus,
  mcpUrl,
  type McpStatus,
} from "./mcp";

const moved: McpStatus = {
  ...DEMO_MCP_STATUS,
  port: 7393,
  url: "http://127.0.0.1:7393/mcp",
  binaryPath: "/Users/me/Apps/Edart.app/Contents/MacOS/edart",
};

const setup = (status: McpStatus | null, id: string) =>
  mcpSetups(status).find((s) => s.id === id)!.text;

describe("MCP bridge", () => {
  it("refuses with a useful message outside the Tauri window", async () => {
    await expect(mcpStatus()).rejects.toThrow(/npm run dev/);
    await expect(mcpSetEnabled(false)).rejects.toThrow(/npm run dev/);
  });
});

describe("MCP setup text", () => {
  it("uses the usual address until the app reports its own", () => {
    expect(mcpUrl(null)).toBe("http://127.0.0.1:7391/mcp");
    expect(mcpUrl({ ...DEMO_MCP_STATUS, listening: false, url: null })).toBe(
      "http://127.0.0.1:7391/mcp",
    );
    expect(mcpUrl(moved)).toBe("http://127.0.0.1:7393/mcp");
  });

  it("gives Claude Code a command to run", () => {
    expect(setup(null, "claude-code")).toBe(
      "claude mcp add --transport http edart http://127.0.0.1:7391/mcp",
    );
    expect(setup(moved, "claude-code")).toContain("http://127.0.0.1:7393/mcp");
  });

  it("gives Claude Desktop the stdio bridge, at the app's real path", () => {
    expect(JSON.parse(setup(null, "claude-desktop"))).toEqual({
      mcpServers: {
        edart: {
          command: "/Applications/Edart.app/Contents/MacOS/edart",
          args: ["--mcp-stdio"],
        },
      },
    });
    expect(
      JSON.parse(setup(moved, "claude-desktop")).mcpServers.edart.command,
    ).toBe("/Users/me/Apps/Edart.app/Contents/MacOS/edart");
  });

  it("gives Cursor the address as a url", () => {
    expect(JSON.parse(setup(moved, "cursor"))).toEqual({
      mcpServers: { edart: { url: "http://127.0.0.1:7393/mcp" } },
    });
    expect(setup(moved, "other")).toBe("http://127.0.0.1:7393/mcp");
  });
});

describe("MCP headline", () => {
  it("says where it listens, that it is off, or why it failed", () => {
    expect(mcpHeadline(null).tone).toBe("wait");
    expect(mcpHeadline(DEMO_MCP_STATUS)).toEqual({
      tone: "ok",
      text: "Listening on http://127.0.0.1:7391/mcp",
    });
    const off = { ...DEMO_MCP_STATUS, enabled: false, listening: false };
    expect(mcpHeadline({ ...off, port: null, url: null })).toEqual({
      tone: "off",
      text: "Off",
    });
    const failed = {
      ...DEMO_MCP_STATUS,
      listening: false,
      port: null,
      url: null,
      error: "Could not listen on ports 7391 to 7399: address in use",
    };
    expect(mcpHeadline(failed)).toEqual({ tone: "bad", text: failed.error });
  });

  it("formats a call's time as a clock", () => {
    expect(mcpCallTime(new Date(2026, 10, 19, 9, 5, 7).getTime())).toBe(
      "09:05:07",
    );
  });
});
