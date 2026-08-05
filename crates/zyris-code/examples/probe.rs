//! Diagnostic. Prints the agent list with UUID versions and tries sending one turn to the given agent.
//!
//! A UUID version nibble of 4 means a DB row; 5 means a git agent synthesized from the content repo.
//!
//! ```bash
//! ZYRIS_PROFILE=zyris-code cargo run -p zyris-code --example probe            # list only
//! ZYRIS_PROFILE=zyris-code cargo run -p zyris-code --example probe -- <name>  # send to that agent
//! ```

use std::process::ExitCode;
use std::time::Duration;

use zyris::runtime::Runner;
use zyris::{Connection, NodeKind};
use zyris_attacca::{AttaccaApi, AttaccaApiClient, ZNewSession};

const WAIT: Duration = Duration::from_secs(5);

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt().with_env_filter("probe=info,zyris=warn").init();

    Runner::from_env()
        .kind(NodeKind::Service)
        .request_scopes(["agents:read", "sessions:read", "sessions:write"])
        .on_connect(|conn| async move {
            probe(&conn).await;
            std::process::exit(0);
        })
        .run()
        .await
}

async fn probe(conn: &Connection) {
    let api = match conn.wait_capability::<AttaccaApiClient>(WAIT).await {
        Ok(api) => api,
        Err(e) => {
            println!("attacca_api 없음: {e}");
            return;
        }
    };

    let agents = match api.list_agents().await {
        Ok(a) => a,
        Err(e) => {
            println!("list_agents 실패: {e}");
            return;
        }
    };

    println!("\n=== 에이전트 {}개 ===", agents.len());
    for a in &agents {
        // The 13th hex character of the UUID is the version: the first character of the third group in 8-4-4-4-12.
        let version = a.id.split('-').nth(2).and_then(|g| g.chars().next()).unwrap_or('?');
        let kind = match version {
            '4' => "DB 행",
            '5' => "git 합성",
            _ => "?",
        };
        println!("  {:38}  v{}  {:9}  {}", a.id, version, kind, a.name);
    }

    let Some(target) = std::env::args().nth(1) else {
        println!("\n(전송 대상 이름을 인자로 주면 한 턴 보내 본다)");
        return;
    };

    let Some(agent) = agents.iter().find(|a| a.name == target) else {
        println!("\n'{target}'이라는 에이전트가 없다");
        return;
    };

    println!("\n=== '{}' 로 전송 시도 ===", agent.name);
    let session = match api
        .create_session_with(ZNewSession {
            agent_id: agent.id.clone(),
            title: None,
            project_id: None,
            preamble: None,
        })
        .await
    {
        Ok(s) => {
            println!("  create_session_with ✓  session_id={}", s.id);
            s
        }
        Err(e) => {
            println!("  create_session_with ✗  {e}");
            return;
        }
    };

    match api.send_message(session.id.clone(), "2 더하기 3은?".into(), vec![]).await {
        Ok(()) => println!("  send_message ✓  전송됐다"),
        Err(e) => println!("  send_message ✗  {e}"),
    }
}
