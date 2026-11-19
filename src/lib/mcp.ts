/**
 * The MCP connector's part of the bridge to Rust: its status, its switch,
 * and the setup text each AI tool needs. Kept apart from `native.ts` so
 * the connector can be mounted with one import; it follows the same rule
 * (components never call `invoke` themselves).
 */

import { invoke } from "@tauri-apps/api/core";

import { nativeAvailable } from "./native";

async function call<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (!nativeAvailable()) {
    throw new Error(
      "The native side is not running. Start the app with `npm run dev`.",
    );
  }
  return invoke<T>(command, args);
}

/** Mirrors `edart_core::mcp::ToolInfo`. */
export interface McpTool {
  name: string;
  description: string;
}

/** Mirrors `edart_core::mcp::CallRecord`. */
export interface McpCall {
  /** Milliseconds since the Unix epoch. */
  atMs: number;
  tool: string;
  ok: boolean;
  error: string | null;
}

/** Mirrors `edart_core::mcp::McpStatus`. */
export interface McpStatus {
  /** The user's choice. */
  enabled: boolean;
  /** Whether the server is accepting connections. */
  listening: boolean;
  port: number | null;
  /** The address to give an MCP client, when listening. */
  url: string | null;
  /** Why the server is not listening although it is enabled. */
  error: string | null;
  /** The app's executable, for clients that launch `--mcp-stdio`. */
  binaryPath: string | null;
  tools: McpTool[];
  /** Recent tool calls, oldest first. */
  calls: McpCall[];
}

export const mcpStatus = () => call<McpStatus>("mcp_status");
/** Turn the connector on or off. Returns the new status. */
export const mcpSetEnabled = (enabled: boolean) =>
  call<McpStatus>("mcp_set_enabled", { enabled });

/** Mirrors `edart_core::mcp::http::DEFAULT_PORT`. */
export const MCP_DEFAULT_PORT = 7391;
/** Where the app is once it has been dragged to Applications. */
export const MCP_DEFAULT_BINARY =
  "/Applications/Edart.app/Contents/MacOS/edart";

/** The address to show: the live one, else the usual one. */
export function mcpUrl(status: McpStatus | null): string {
  return status?.url ?? `http://127.0.0.1:${MCP_DEFAULT_PORT}/mcp`;
}

/** Ready-to-paste setup for one AI tool. */
export interface McpSetup {
  id: "claude-code" | "claude-desktop" | "cursor" | "other";
  label: string;
  /** What to do with `text`. */
  how: string;
  text: string;
}

export function mcpSetups(status: McpStatus | null): McpSetup[] {
  const url = mcpUrl(status);
  const binary = status?.binaryPath ?? MCP_DEFAULT_BINARY;
  const json = (value: unknown) => JSON.stringify(value, null, 2);
  return [
    {
      id: "claude-code",
      label: "Claude Code",
      how: "Run this in a terminal.",
      text: `claude mcp add --transport http edart ${url}`,
    },
    {
      id: "claude-desktop",
      label: "Claude Desktop",
      how: "In Claude: Settings, Developer, Edit Config. Add this to claude_desktop_config.json, then restart Claude.",
      text: json({
        mcpServers: { edart: { command: binary, args: ["--mcp-stdio"] } },
      }),
    },
    {
      id: "cursor",
      label: "Cursor",
      how: "Add this to ~/.cursor/mcp.json.",
      text: json({ mcpServers: { edart: { url } } }),
    },
    {
      id: "other",
      label: "Other",
      how: "Any MCP client that supports Streamable HTTP: give it this address.",
      text: url,
    },
  ];
}

/** The one-line state of the connector, and the dot's colour. */
export function mcpHeadline(status: McpStatus | null): {
  tone: "ok" | "off" | "bad" | "wait";
  text: string;
} {
  if (!status) return { tone: "wait", text: "Checking…" };
  if (status.listening && status.url)
    return { tone: "ok", text: `Listening on ${status.url}` };
  if (!status.enabled) return { tone: "off", text: "Off" };
  return { tone: "bad", text: status.error ?? "Not listening" };
}

/** "14:02:07", in the Mac's own time zone. */
export function mcpCallTime(atMs: number): string {
  const at = new Date(atMs);
  const two = (n: number) => String(n).padStart(2, "0");
  return `${two(at.getHours())}:${two(at.getMinutes())}:${two(at.getSeconds())}`;
}

/** Sample data for `?demo`, in a browser tab. */
export const DEMO_MCP_STATUS: McpStatus = {
  enabled: true,
  listening: true,
  port: MCP_DEFAULT_PORT,
  url: `http://127.0.0.1:${MCP_DEFAULT_PORT}/mcp`,
  error: null,
  binaryPath: MCP_DEFAULT_BINARY,
  tools: [
    {
      name: "get_status",
      description: "Report which iPhone is connected and how.",
    },
    { name: "screenshot", description: "Take a picture of the screen." },
    { name: "tap", description: "Tap a point of the last screenshot." },
    { name: "double_tap", description: "Tap twice quickly." },
    { name: "long_press", description: "Press and hold." },
    { name: "swipe", description: "Drag from one point to another." },
    { name: "type_text", description: "Type text as a keyboard would." },
    { name: "press_home", description: "Go to the Home Screen." },
    { name: "wait", description: "Pause while the screen changes." },
  ],
  calls: [
    { atMs: 1795096921000, tool: "get_status", ok: true, error: null },
    { atMs: 1795096923000, tool: "screenshot", ok: true, error: null },
    { atMs: 1795096927000, tool: "tap", ok: true, error: null },
    { atMs: 1795096929000, tool: "screenshot", ok: true, error: null },
    {
      atMs: 1795096934000,
      tool: "tap",
      ok: false,
      error:
        "(1320, 400) is outside the screenshot, which is 1170 by 2532 pixels.",
    },
    { atMs: 1795096938000, tool: "type_text", ok: true, error: null },
  ],
};
