// W release na Windows bez okna konsoli; stdio przekazane pipe'ami przez klienta MCP działa normalnie.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args().nth(1).as_deref() == Some(ecommerce_mcp::MCP_ARG) {
        std::process::exit(ecommerce_mcp::mcp::run_stdio());
    }
    ecommerce_mcp::app::run();
}
