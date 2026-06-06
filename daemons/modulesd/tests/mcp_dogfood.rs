//! End-to-end test for the `mcp.server` Tier 1 dogfood module
//! `examples/modules/com.example.mcp-demo`.
//!
//! Verifies the full chain: manifest discovery → `[mcp.server]`
//! recognition → instantiation → `Guest::init` → modulesd's MCP
//! socket bridge → an `McpClient` (the AI daemon's real client)
//! connecting, listing the `echo` tool, and calling it.
//!
//! The test skips itself when `module.wasm` is absent, the same way
//! `tier1_dogfood.rs` does: building the component needs
//! `cargo-component` and the `wasm32` target, which are not part of
//! the default test environment.

use std::path::Path;

use arlen_ai_core::mcp::{CallChain, McpClient, ServerClass, ServerId};
use arlen_modulesd::manager::Manager;
use arlen_modulesd::runtime::mcp::mcp_module_socket_path;
use tokio::sync::broadcast;

fn copy_dir_recursive(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap();
        }
    }
}

fn example_module_path() -> std::path::PathBuf {
    let mut p = std::env::current_dir().unwrap();
    p.push("../examples/modules/com.example.mcp-demo");
    p
}

#[tokio::test]
async fn mcp_demo_module_lists_and_calls_echo() {
    let example = example_module_path();
    let wasm = example.join("module.wasm");

    if !example.exists() {
        eprintln!(
            "skipping: example module dir not present at {}",
            example.display()
        );
        return;
    }
    if !wasm.exists() {
        eprintln!(
            "skipping: module.wasm absent at {}. Build with `cargo install cargo-component` + `cargo component build --release` in the example dir.",
            wasm.display()
        );
        return;
    }

    // Isolate both the module search path and the runtime dir (which
    // is where the MCP socket is bound) into temp directories.
    let modules_dir = tempfile::tempdir().unwrap();
    let runtime_dir = tempfile::tempdir().unwrap();
    std::env::set_var("LUNARIS_USER_MODULES_DIR", modules_dir.path());
    std::env::set_var("XDG_RUNTIME_DIR", runtime_dir.path());
    copy_dir_recursive(&example, &modules_dir.path().join("com.example.mcp-demo"));

    let (tx, _rx) = broadcast::channel(16);
    let manager = Manager::new(tx).unwrap();
    manager.discover().await;
    // Hosts the MCP socket and waits for the bind to land.
    manager.start_all_mcp_servers().await;

    let socket = mcp_module_socket_path("com.example.mcp-demo");
    assert!(
        socket.exists(),
        "mcp socket was not bound at {}",
        socket.display()
    );

    // Connect with the AI daemon's real MCP client.
    let mut client = McpClient::new();
    let id = ServerId("com.example.mcp-demo".to_string());
    client
        .connect(id.clone(), &socket.to_string_lossy(), ServerClass::Action)
        .await
        .expect("McpClient connect");

    // tools/list surfaces the module's one tool.
    let tools = client.list_tools(&id).await.expect("list tools");
    assert!(
        tools.iter().any(|t| t.name == "echo"),
        "echo tool not exposed; got: {:?}",
        tools.iter().map(|t| &t.name).collect::<Vec<_>>()
    );

    // tools/call round-trips the argument through the WASM guest.
    let chain = CallChain::root();
    let result = client
        .call_tool(
            &id,
            "echo",
            serde_json::json!({ "text": "arlen" }),
            &chain,
        )
        .await
        .expect("call echo");
    assert!(
        result.contains("arlen"),
        "echo result did not carry the input: {result}"
    );

    // Revocation: once the module is stopped, the connection the
    // client still holds open must fail closed on the next call.
    manager.stop_mcp_server("com.example.mcp-demo").await;
    let after_stop = client
        .call_tool(
            &id,
            "echo",
            serde_json::json!({ "text": "after" }),
            &CallChain::root(),
        )
        .await;
    assert!(
        after_stop.is_err(),
        "tool call succeeded after the module was stopped: {after_stop:?}"
    );

    manager.shutdown_all_mcp().await;
    std::env::remove_var("LUNARIS_USER_MODULES_DIR");
    std::env::remove_var("XDG_RUNTIME_DIR");
}
