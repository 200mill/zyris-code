//! **`wait`가 라이브에서 실제로 기다리는가.**
//!
//! 로컬 테스트가 다 초록이어도 이건 라이브에서만 드러난다 — 이 리포에서 와이어 이름으로
//! 두 번 그랬고, 두 번 다 로컬은 초록이었다. 여기서 재는 것은 딱 하나다:
//! **60초 마감을 넘겨 도는 명령을 에이전트가 실패로 읽지 않고 끝까지 기다리는가.**
//!
//! `announce_probe`와 같은 규칙을 따른다 — **에이전트의 말은 근거로 쓰지 않는다.**
//! 도구가 있는데도 "없습니다"라고 단언하는 것을 네 번 겪었다. 판정은 셋이고 전부
//! 부작용이거나 시계다:
//!
//! 1. **표시 파일이 실제로 생겼는가.** 배경 명령이 끝까지 돌았다는 뜻이다.
//! 2. 턴이 **와이어 마감(60초)보다 오래 살아 있었는가.** 한 번의 노드 호출은 50초에
//!    돌아오므로, 그보다 오래 걸린 일을 끝냈다면 `until`을 여러 번 부른 것이다.
//! 3. 에이전트의 답에 실패를 말하는 낱말이 없는가. **이것만은 보조 근거다** — 셋 중
//!    앞의 둘이 통과했는데 이것만 걸리면 문구 문제이지 동작 문제가 아니다.
//!
//! ```bash
//! timeout 900 cargo run -j2 -p zyris-code --example wait_probe
//! ```

use std::process::ExitCode;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use zyris::runtime::Runner;
use zyris::{Connection, NodeKind};
use zyris_attacca::{AttaccaApi, AttaccaApiClient, ZDeltaKind, ZNewSession, ZTurnFrame};

const CONSUME_WAIT: Duration = Duration::from_secs(5);
/// 배경에서 돌릴 시간. **와이어 마감(60초)보다 넉넉히 길어야** 재는 뜻이 있다.
const SLEEP_SECS: u64 = 90;

/// 시킬 말. `$ZYRIS_CODE_PROBE_ASK`로 바꾼다.
///
/// **도구 이름을 실제 와이어 모양으로 말해 준다.** `wait.start`라고만 부르면 모델이
/// 그 이름을 못 찾고 "도구가 없다"로 포기한다 — `announce_probe`에서 실제로 그랬다.
fn ask(marker: &std::path::Path) -> String {
    std::env::var("ZYRIS_CODE_PROBE_ASK").unwrap_or_else(|_| {
        format!(
            "오래 걸리는 일을 하나 시킨다. \
             (1) 이름이 '__wait__start'로 끝나는 도구를 \
             command='sleep {SLEEP_SECS}; date > {}'로 호출하라. \
             (2) 그다음 이름이 '__wait__until'로 끝나는 도구를 그 job id로 호출하라. \
             done이 false로 오면 **실패가 아니라 아직 안 끝난 것이므로** 같은 인자로 \
             다시 호출하라. done이 true가 될 때까지 반복하라. \
             (3) 끝나면 종료 코드를 한 줄로 알려라.",
            marker.display()
        )
    })
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "zyris=warn".into()),
        )
        .init();

    // TUI와 같은 노드 신원을 쓴다. 다른 프로필로 붙으면 등록 코드를 다시 요구한다.
    if std::env::var_os("ZYRIS_PROFILE").is_none() {
        std::env::set_var("ZYRIS_PROFILE", "zyris-code");
    }

    // 진짜 리포를 만지게 두지 않는다. 임시 디렉터리 하나가 이 프로브의 세계다.
    let dir = std::env::temp_dir().join("zyris-code-wait-probe");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("임시 디렉터리");
    let marker = dir.join("done.txt");
    println!("작업 디렉터리: {}\n", dir.display());

    // 프로브에는 화면이 없다. 게이트가 물을 곳이 없으므로 **자동 모드로 맞춰 둔다** —
    // 승인 경로가 아니라 기다리는 경로를 재는 것이 이 프로브의 일이다.
    let bridge = zyris_code::tools::bridge::Bridge::new();
    bridge.sync(zyris_code::mode::Mode::Job, &Default::default());

    let (_api_tx, api_rx) = tokio::sync::watch::channel(None);
    zyris_code::tools::announce(Runner::from_env(), dir, bridge, api_rx)
        .kind(NodeKind::Service)
        .request_scopes(["agents:read", "sessions:write", "events:read"])
        .on_connect(move |conn| {
            let marker = marker.clone();
            async move {
                let ok = match ask_the_agent(&conn, &marker).await {
                    Ok(ok) => ok,
                    Err(e) => {
                        println!("\n### 프로브 실패: {e}\n");
                        false
                    }
                };
                std::process::exit(i32::from(!ok));
            }
        })
        .run()
        .await
}

async fn ask_the_agent(conn: &Connection, marker: &std::path::Path) -> anyhow::Result<bool> {
    let announced = conn.announce().await?;
    println!("announce → accepted={:?} rejected={:?}", announced.accepted, announced.rejected);

    // 서버는 handshake 뒤 500ms 안에 capability를 스냅숏한다. 넉넉히 기다려 그것이
    // 원인인지부터 배제한다.
    let wait: u64 =
        std::env::var("ZYRIS_CODE_PROBE_WAIT").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
    println!("{wait}초 기다린다");
    tokio::time::sleep(Duration::from_secs(wait)).await;

    let api = conn.wait_capability::<AttaccaApiClient>(CONSUME_WAIT).await?;

    let wanted = std::env::var("ZYRIS_CODE_AGENT").unwrap_or_else(|_| "Main Agent".into());
    let agent = api
        .list_agents()
        .await?
        .into_iter()
        .find(|a| a.name == wanted)
        .ok_or_else(|| anyhow::anyhow!("'{wanted}' 에이전트가 없다"))?;
    println!("에이전트: {} ({})", agent.name, agent.id);

    let session = api
        .create_session_with(ZNewSession {
            agent_id: agent.id.clone(),
            title: None,
            project_id: None,
            preamble: None,
        })
        .await?;
    let ask = ask(marker);
    println!("세션: {}\n묻는다: {ask}\n", session.id);

    let at = Instant::now();
    api.send_message(session.id.clone(), ask, Vec::new()).await?;

    let mut stream = api.turn_events(session.id.clone(), None).await?;
    let mut answer = String::new();
    // **턴이 시작하기 전의 `running:false`를 끝으로 오해하면 안 된다.** 구독이 전송보다
    // 늦게 붙으면 첫 상태가 false로 오고, 거기서 끊으면 답이 늘 비어 보인다.
    let mut started = false;
    while let Some(frame) = stream.items.next().await {
        match frame? {
            ZTurnFrame::Delta { kind: ZDeltaKind::Assistant, text } => {
                started = true;
                print!("{text}");
                use std::io::Write;
                let _ = std::io::stdout().flush();
                answer.push_str(&text);
            }
            ZTurnFrame::Status { running: true } => started = true,
            ZTurnFrame::Status { running: false } if started => break,
            _ => {}
        }
    }
    let took = at.elapsed();

    println!("\n\n=== 판정 1: 배경 명령이 끝까지 돌았는가 ===");
    let landed = marker.exists();
    match std::fs::read_to_string(marker) {
        Ok(body) => println!("통과 — 표시 파일이 생겼다: {}", body.trim()),
        Err(e) => println!("실패 — 표시 파일이 없다({e}). 명령이 중간에 죽었다는 뜻이다."),
    }

    println!("\n=== 판정 2: 마감을 넘겨 살아 있었는가 ===");
    // 한 번의 노드 호출은 50초 안에 돌아온다. 그보다 오래 걸린 일을 끝냈다면
    // `until`을 여러 번 부른 것이다 — 그것이 이 캐퍼빌리티의 존재 이유다.
    let survived = took > Duration::from_secs(60);
    println!(
        "{} — 턴이 {}초 걸렸다(배경 명령 {SLEEP_SECS}초).",
        if survived { "통과" } else { "실패" },
        took.as_secs()
    );

    println!("\n=== 판정 3: 실패로 읽지 않았는가 (보조) ===");
    let lower = answer.to_lowercase();
    let bad = ["실패", "timed out", "timeout", "시간 초과", "취소"];
    let complained: Vec<&str> = bad.into_iter().filter(|w| lower.contains(w)).collect();
    if complained.is_empty() {
        println!("통과 — 답에 실패를 말하는 낱말이 없다.");
    } else {
        println!("걸림 — 답에 {complained:?}가 있다. 문구를 다시 볼 것:\n{}", answer.trim());
    }

    let ok = landed && survived;
    println!("\n=== 판정: {} ===", if ok { "통과" } else { "실패" });
    Ok(ok)
}
