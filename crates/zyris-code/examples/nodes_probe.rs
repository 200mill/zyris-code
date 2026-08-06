//! Diagnostic. **Measures whether the server splits nodes when the same credentials connect twice.**
//!
//! This question decides zyris-code's whole multi-window design. If the server gives a different
//! `node_id` per connection, windows can each just connect (`sibling.rs`'s sibling-node cloning
//! becomes unnecessary); same `node_id` means the later connection takes over all the earlier one's tool calls.
//!
//! **Don't guess.** It's confirmed that attacca answers `register_node` with `MethodNotFound`
//! (2026-08-03), so this is the only road left and it's actually re-tested here.
//!
//! ```bash
//! ZYRIS_PROFILE=zyris-code cargo run -p zyris-code --example nodes_probe
//! ```
//!
//! The point is that `node_id` is **the value the server gave in the ack** (`ack.node_id` in `connection.rs`).

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use zyris::runtime::{credentials::Credentials, RunConfig, Runner};
use zyris::NodeKind;

/// To see whether the second connection displaces the first, it must connect while the first is alive.
const OVERLAP: Duration = Duration::from_secs(12);

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt().with_env_filter("nodes_probe=info,zyris=warn").init();

    // Must look at the **same credential file** as the app. Same spot `main.rs` uses.
    if std::env::var_os("ZYRIS_CONFIG_DIR").is_none() {
        if let Some(dir) = zyris_code::conn::credential_dir() {
            std::env::set_var("ZYRIS_CONFIG_DIR", dir);
        }
    }
    if std::env::var_os("ZYRIS_PROFILE").is_none() {
        std::env::set_var("ZYRIS_PROFILE", zyris_code::conn::APP);
    }
    if std::env::var_os("ZYRIS_NODE_NAME").is_none() {
        std::env::set_var("ZYRIS_NODE_NAME", zyris_code::conn::node_name());
    }

    let config = RunConfig::from_env();
    let bridge = zyris_code::tools::bridge::Bridge::new();
    let creds: Arc<dyn Credentials> = match zyris_code::enroll::source(&config, &bridge) {
        Ok((creds, _)) => creds,
        Err(e) => {
            println!("could not build credentials: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Two share one credential — the same shape as two windows reading the same file.
    let first = dial("first", creds.clone());
    tokio::time::sleep(Duration::from_secs(3)).await;
    let second = dial("second", creds.clone());

    tokio::time::sleep(OVERLAP).await;
    println!("\nboth connections overlapped. Compare the node_id on the two lines above:");
    println!("  different → the server splits nodes per connection (each window can connect)");
    println!("  same      → they fight over one node (the later one takes the tool calls)");
    first.abort();
    second.abort();
    ExitCode::SUCCESS
}

fn dial(label: &'static str, creds: Arc<dyn Credentials>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let runner = Runner::new(RunConfig::from_env(), creds).kind(NodeKind::Service).on_connect(
            move |conn| async move {
                let info = conn.info();
                println!("{label}: node_id={} conn_id={}", info.node_id, info.conn_id);
                // Must stay connected for the overlap. If it drops, there's no way to see displacement.
                conn.closed().await;
                println!("{label}: connection dropped");
            },
        );
        let _ = runner.try_run().await;
    })
}
