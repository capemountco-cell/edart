# The MCP connector

Edart runs a small [Model Context Protocol](https://modelcontextprotocol.io)
server so AI tools on this Mac (Claude, Claude Code, Cursor, any MCP client)
can look at the iPhone's screen and tap, swipe and type on it.

It starts with the app and listens at:

```
http://127.0.0.1:7391/mcp
```

If port 7391 is taken, Edart tries 7392 to 7399 and shows the one it got in
the Connectors card. The card also has a switch to turn the server off; the
choice is remembered.

## Setting up a client

Edart must be open for any of these to reach the phone.

**Claude Code**

```sh
claude mcp add --transport http edart http://127.0.0.1:7391/mcp
```

**Claude Desktop** launches local commands, so it uses the app's stdio
bridge. In Claude: Settings, Developer, Edit Config, then add to
`claude_desktop_config.json` and restart Claude:

```json
{
  "mcpServers": {
    "edart": {
      "command": "/Applications/Edart.app/Contents/MacOS/edart",
      "args": ["--mcp-stdio"]
    }
  }
}
```

`edart --mcp-stdio` opens no window. It forwards each message to the running
app and prints the reply. If Edart is not open, the client still connects
and sees the tools, and every tool call answers "Edart isn't open. Open the
Edart app on this Mac, then try again." It finds the app again by itself
once it is open.

**Cursor**: add to `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "edart": { "url": "http://127.0.0.1:7391/mcp" }
  }
}
```

**Anything else** that speaks MCP over Streamable HTTP: give it the address.
Clients that only launch commands can use the stdio bridge above.

The Connectors card in the app has a Copy button for each of these, with the
real port and the app's real path filled in.

## Tools

Coordinates are pixels of the most recent `screenshot`, from its top-left
corner. A client must take a screenshot before it can tap or swipe, and
points outside that screenshot are refused.

| Tool         | Arguments                                                        | What it does                                                                             |
| ------------ | ---------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `get_status` | none                                                             | Phone name, control link (`bluetooth`, `usb`, `none`), whether screenshots are available |
| `screenshot` | none                                                             | The screen as a JPEG, plus its size in pixels                                            |
| `tap`        | `x`, `y`                                                         | Tap a point                                                                              |
| `double_tap` | `x`, `y`                                                         | Tap twice quickly                                                                        |
| `long_press` | `x`, `y`, `ms` (optional, up to 10000)                           | Press and hold; 800 ms by default                                                        |
| `swipe`      | `from_x`, `from_y`, `to_x`, `to_y`, `ms` (optional, up to 10000) | Drag between two points; 300 ms by default                                               |
| `type_text`  | `text` (1 to 2000 characters)                                    | Type into whatever has focus; US-keyboard characters only                                |
| `press_home` | none                                                             | Go to the Home Screen                                                                    |
| `wait`       | `ms` (up to 5000)                                                | Pause while the screen changes                                                           |

`type_text`, `press_home`, `wait` and `get_status` work without a screenshot.

A tool that fails (no phone connected, a point off the screen, a character
that cannot be typed) returns a normal result with `isError: true` and a
sentence the model can act on. JSON-RPC errors are kept for malformed
requests: `-32700` invalid JSON, `-32600` invalid request (including
batches, which are not supported), `-32601` unknown method, `-32602` unknown
tool or malformed `tools/call`.

## Protocol

- Streamable HTTP: `POST /mcp` with one JSON-RPC 2.0 message; the reply is
  `application/json`. Notifications get `202 Accepted`. There is no
  server-to-client stream and no session, so `GET` and `DELETE` answer `405`.
- Protocol revisions `2025-06-18` and `2025-03-26`. `initialize` echoes the
  client's revision if it is one of these, else offers `2025-06-18`.
- Capabilities: `tools` only.
- One request per connection (`Connection: close`).

## Security

What Edart does:

- **Loopback only.** The server binds `127.0.0.1`. Nothing on the network,
  not even another device on the same Wi-Fi, can connect.
- **Web pages are refused.** A page open in a browser on this Mac can send
  requests to `127.0.0.1`, so the server refuses any request with an
  `Origin` header that is not `http://localhost`, `http://127.0.0.1` or
  `http://[::1]` (compared exactly, with an optional port), any request
  whose `Host` is not this machine (DNS rebinding), and any body that is not
  `application/json`. It never sends CORS headers.
- **Limits.** Bodies over 1 MB, headers over 16 KB, requests that take more
  than 15 seconds to arrive and more than 16 connections at once are
  refused.
- **Off switch.** Turning the connector off in the app closes the port.

What it does not do:

- **No password.** There is no token or login, because the server cannot be
  reached from outside this Mac. That means any program running under your
  macOS user account (and any other account on this Mac) can use it while
  Edart is open, and so can see your iPhone's screen and act on it. Turn the
  connector off when you do not want that.
- **No confirmation per action.** A connected AI tool can tap anything you
  could. Use your AI tool's own approval settings for tool calls if you want
  to confirm each step.

## For developers

- `crates/edart-core/src/mcp/`: the protocol (`server.rs`), the tools
  (`tools.rs`), the HTTP transport (`http.rs`) and the stdio bridge
  (`stdio.rs`). No Tauri; tested on Linux, including over real HTTP
  (`crates/edart-core/tests/mcp_http.rs`).
- `src-tauri/src/mcp/`: starts the server in the app and gives it the real
  phone. Screen mirroring provides screenshots by calling
  `mcp::set_screenshot_provider`.
- To try a client without a Mac or a phone:
  `cargo run -p edart-core --example mcp_demo`.
