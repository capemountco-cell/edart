// Hides the console window on Windows release builds. Harmless on macOS.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `edart --mcp-stdio`: no window. Bridge an MCP client that can only
    // launch a command to the app that is already running.
    if std::env::args().any(|arg| arg == "--mcp-stdio") {
        edart_core::mcp::stdio::run_process();
        return;
    }
    edart_lib::run();
}
