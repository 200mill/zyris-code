//! 기다리는 도구. **배경 실행 도구가 아니다** — 로컬 빌드도, 원격 빌드도, attacca의
//! work도 같은 도구로 기다린다.
//!
//! **이 파일의 전부는 `until`의 반환 계약이다.** 끝났든 안 끝났든 성공으로 답하고,
//! 와이어 마감 안쪽에 돌아오고, 안 끝났으면 다시 부르라고 말한다.
//!
//! 고치려는 것은 이것이다: `terminal.exec`이 50초에 잘리면서 프로세스 트리까지 죽이고
//! `timed_out: true, exit_code: -1`을 주면, 에이전트는 그것을 **실패로 읽고 멈춘다.**
//! 그 판단은 에이전트가 옳다 — 고칠 것은 그 모양을 만들어 보내는 쪽이다.

use std::path::PathBuf;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use zyris::WireError;
// `AttaccaApi`는 트레이트다. 클라이언트만 임포트하면 메서드가 전부 "method not found"다.
use zyris_attacca::{AttaccaApi, AttaccaApiClient};

use crate::tools::jobs::{Chunk, Jobs, Snapshot, Spec};

/// 한 번의 `logs`가 주는 최대 크기. 남은 것은 `more`로 말한다.
const LOGS_BYTES: usize = 16_000;
/// 결과에 싣는 출력 꼬리. **매번 전문을 실으면 컨텍스트가 남아나지 않는다** —
/// 20분 빌드면 확인이 25번쯤 오간다. 전문은 `wait.logs`로 가져간다.
const TAIL_BYTES: usize = 2000;
/// 우리가 답을 만들어 보낼 여유. 마감에서 이만큼 뺀 것이 한 번의 예산이다.
const HEADROOM: std::time::Duration = std::time::Duration::from_secs(5);
/// 마감이 꺼져 있을 때의 상한. 그때는 저쪽이 안 끊는다는 뜻이지만 영원히 잡고 있을 수는 없다.
const NO_DEADLINE_CAP: std::time::Duration = std::time::Duration::from_secs(600);
/// 되묻는 기본 간격.
const PROBE_EVERY_MS: u64 = 5_000;
/// 그 바닥. **되물을 명령은 싸야 한다** — `cargo build`를 되묻기로 거는 사고를 막는다.
const PROBE_FLOOR_MS: u64 = 2_000;
/// 되묻는 명령 하나의 제한. 넘으면 그 회차는 거짓으로 친다.
const PROBE_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

/// attacca work를 되묻는 간격. **저쪽은 알림이 없어 폴링뿐이다.**
const WORK_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

fn probe_gap(every_ms: Option<u64>) -> std::time::Duration {
    std::time::Duration::from_millis(every_ms.unwrap_or(PROBE_EVERY_MS).max(PROBE_FLOOR_MS))
}

/// work가 **사람이나 에이전트가 움직여야 하는 자리**에 닿았는가.
///
/// 관문 둘·멈춤·끝이다. `Planning`·`Executing`은 아직 저쪽이 도는 중이라, 그때
/// 깨우면 에이전트는 할 일이 없는데 돌아와 확인만 되풀이한다.
///
/// **일부러 남김없이 적는다** — 상류에 상태가 하나 늘면 여기서 컴파일이 막혀야
/// 새 상태를 어느 쪽으로 볼지 정하게 된다.
fn work_needs_someone(state: zyris_attacca::ZWorkState) -> bool {
    use zyris_attacca::ZWorkState::*;
    match state {
        CheckingRequirements | Planning | Executing | Verifying => false,
        Draft | AwaitingGoalApproval | AwaitingPlanApproval | Halted | Done | Failed
        | Cancelled => true,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Started {
    /// Pass this to `wait.until`, `wait.logs` and `wait.stop`.
    pub id: String,
    pub label: String,
    /// What to do next. Follow it.
    pub next: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JobRef {
    pub id: String,
    pub label: String,
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Logs {
    pub text: String,
    /// Pass this back as `offset` to read on from here.
    pub next_offset: u64,
    /// More is buffered. **Call again and you get it.**
    pub more: bool,
    /// Bytes lost for good to overflow. Calling again will not bring them back.
    pub dropped: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[zyris::capability(name = "wait", version = 1)]
pub trait Wait {
    /// Run a command in the background and return at once. Use this for anything that might
    /// take more than a minute — builds, test suites, deploys — then wait with `until`.
    ///
    /// Give exactly one of `command` (a shell one-liner) or `argv` (a program and its
    /// arguments, run without a shell). `cwd` resolves against the working directory unless
    /// it starts with `/`. `env` takes `KEY=VALUE` lines. `label` is what a person sees on
    /// screen; it defaults to the command.
    async fn start(
        &self,
        command: Option<String>,
        argv: Option<Vec<String>>,
        cwd: Option<String>,
        env: Option<Vec<String>>,
        label: Option<String>,
    ) -> zyris::Result<Started>;

    /// Background jobs from this session, oldest first.
    async fn list(&self) -> zyris::Result<Vec<JobRef>>;

    /// A job's output from `offset` (0 for the beginning). Output stays readable after the
    /// job has ended, so you can read the whole thing once `until` says it is done.
    async fn logs(&self, job: String, offset: Option<u64>) -> zyris::Result<Logs>;

    /// Wait until something finishes, answering within the call deadline. **Not finishing is
    /// not an error** — when `done` is false, call again with the same arguments.
    ///
    /// Give exactly one of: `job` (a background job from `start`), `command` (re-run it until
    /// it exits 0, or until its output matches `matches`), or `work` (an attacca work id).
    /// `every_ms` is the gap between re-runs of `command`, at least 2000. `timeout_ms` caps
    /// this one call; it is trimmed to the deadline, never past it.
    async fn until(
        &self,
        job: Option<String>,
        command: Option<String>,
        work: Option<String>,
        matches: Option<String>,
        every_ms: Option<u64>,
        timeout_ms: Option<u64>,
    ) -> zyris::Result<Outcome>;

    /// Kill a background job and everything it started. A job that already ended is fine.
    async fn stop(&self, job: String) -> zyris::Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Outcome {
    /// True when the thing you waited for has finished. **False is not a failure** — it
    /// means not yet.
    pub done: bool,
    /// What happened, in one line.
    pub why: String,
    /// What to do next. When `done` is false this tells you to call again.
    pub next: String,
    pub elapsed_ms: u64,
    /// The last few lines of output, when there are any. Read it all with `logs`.
    pub tail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

/// 이 한 번의 호출이 쓸 수 있는 시간.
///
/// **attacca가 노드 호출을 60초에 오류로 끊는다.** 그 전에 우리가 성공으로 답해야
/// 에이전트가 실패로 읽지 않는다. 마감은 상한이지 기본값이 아니라, 에이전트가 짧게
/// 잡았으면 그쪽이 이긴다.
fn budget(deadline: Option<std::time::Duration>, timeout_ms: Option<u64>) -> std::time::Duration {
    let asked = timeout_ms.map(std::time::Duration::from_millis);
    match deadline {
        Some(d) => {
            let room = d.saturating_sub(HEADROOM).max(std::time::Duration::from_secs(1));
            asked.map(|a| a.min(room)).unwrap_or(room)
        }
        None => asked.unwrap_or(NO_DEADLINE_CAP),
    }
}

/// `Jobs`와 attacca 손잡이를 들고 있는 구현.
///
/// **손잡이는 붙은 뒤에 온다** — 도구는 붙기 전에 announce되므로 `watch`로 받아 두고
/// 부를 때 집는다. `work.rs`의 `Works`와 같은 수법이다.
#[derive(Clone)]
pub struct Waits {
    pub(crate) jobs: Jobs,
    api: watch::Receiver<Option<Arc<AttaccaApiClient>>>,
}

impl Waits {
    pub fn new(jobs: Jobs, api: watch::Receiver<Option<Arc<AttaccaApiClient>>>) -> Waits {
        Waits { jobs, api }
    }

    pub(crate) fn api(&self) -> Result<Arc<AttaccaApiClient>, WireError> {
        self.api.borrow().clone().ok_or_else(|| {
            WireError::internal("아직 attacca에 붙지 않았습니다. 잠시 뒤에 다시 불러 주세요.")
        })
    }

    /// 모르는 작업은 **오류다.** 조용히 빈 결과를 주면 도구가 고장 난 줄 알고
    /// 에이전트가 다른 길로 같은 일을 시도한다.
    fn known(&self, id: &str) -> Result<Snapshot, WireError> {
        self.jobs.snapshot(id).ok_or_else(|| {
            WireError::invalid_params(format!(
                "`{id}`이라는 배경 작업이 없습니다. wait.list로 확인해 주세요."
            ))
        })
    }
}

#[async_trait::async_trait]
impl Wait for Waits {
    async fn start(
        &self,
        command: Option<String>,
        argv: Option<Vec<String>>,
        cwd: Option<String>,
        env: Option<Vec<String>>,
        label: Option<String>,
    ) -> zyris::Result<Started> {
        let spec = Spec {
            command,
            argv,
            cwd: cwd.filter(|s| !s.is_empty()).map(PathBuf::from),
            // `KEY=VALUE` 줄로 받는다. 맵을 스키마에 넣으면 모델이 자주 틀린다.
            env: env
                .unwrap_or_default()
                .iter()
                .filter_map(|line| line.split_once('='))
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            label,
        };
        let id = self.jobs.start(spec).map_err(WireError::invalid_params)?;
        let snap = self.known(&id)?;
        Ok(Started {
            id: id.clone(),
            label: snap.label,
            next: format!("걸었습니다. `wait.until`을 `job: \"{id}\"`로 불러 끝나기를 기다리세요."),
        })
    }

    async fn list(&self) -> zyris::Result<Vec<JobRef>> {
        Ok(self.jobs.list().into_iter().map(job_ref).collect())
    }

    async fn logs(&self, job: String, offset: Option<u64>) -> zyris::Result<Logs> {
        let snap = self.known(&job)?;
        let offset = offset.unwrap_or(0);
        let chunk = self.jobs.read(&job, offset).unwrap_or_else(empty_chunk);
        // 이 덩어리가 실제로 시작하는 절대 위치. 버려진 앞을 달라고 했으면 뒤로 밀린다.
        let start = offset + chunk.dropped;
        // 한 번에 다 주지 않는다. **남은 것은 `more`로 말한다** — 잘렸는데 안 말하면
        // 에이전트는 그것이 출력의 전부인 줄 안다.
        let (text, more, next_offset) = if chunk.text.len() > LOGS_BYTES {
            let cut = chunk.text.floor_char_boundary(LOGS_BYTES);
            (chunk.text[..cut].to_string(), true, start + cut as u64)
        } else {
            (chunk.text, false, chunk.next_offset)
        };
        Ok(Logs { text, next_offset, more, dropped: chunk.dropped, exit_code: snap.exit_code })
    }

    async fn until(
        &self,
        job: Option<String>,
        command: Option<String>,
        work: Option<String>,
        matches: Option<String>,
        every_ms: Option<u64>,
        timeout_ms: Option<u64>,
    ) -> zyris::Result<Outcome> {
        let chosen = [job.is_some(), command.is_some(), work.is_some()];
        if chosen.iter().filter(|c| **c).count() != 1 {
            return Err(WireError::invalid_params(
                "job·command·work 중 정확히 하나를 주세요.",
            ));
        }
        let budget = budget(crate::tools::guard::wire_deadline(), timeout_ms);
        let at = std::time::Instant::now();

        if let Some(id) = job {
            return self.until_job(&id, budget, at).await;
        }
        if let Some(line) = command {
            let gap = probe_gap(every_ms);
            return self.until_probe(&line, matches.as_deref(), gap, budget, at).await;
        }
        let work = work.expect("셋 중 하나는 있다");
        self.until_work(&work, budget, at).await
    }

    async fn stop(&self, job: String) -> zyris::Result<()> {
        self.known(&job)?;
        self.jobs.stop(&job);
        Ok(())
    }
}

impl Waits {
    /// 배경 작업이 끝나기를 기다린다. **끝나면 그 즉시 돌아온다.**
    async fn until_job(
        &self,
        id: &str,
        budget: std::time::Duration,
        at: std::time::Instant,
    ) -> zyris::Result<Outcome> {
        let snap = self.known(id)?;
        if !snap.running {
            return Ok(self.finished(id, snap, at));
        }
        let Some(mut ended) = self.jobs.ended(id) else {
            return Ok(self.finished(id, self.known(id)?, at));
        };
        let waited = tokio::time::timeout(budget, async {
            while !*ended.borrow() {
                if ended.changed().await.is_err() {
                    break;
                }
            }
        })
        .await;

        let snap = self.known(id)?;
        if waited.is_ok() && !snap.running {
            return Ok(self.finished(id, snap, at));
        }
        // **여기가 이 파일의 요점이다.** 안 끝났어도 오류가 아니다 — 오류로 주면
        // 에이전트는 빌드가 실패한 줄 알고 멈춘다.
        Ok(Outcome {
            done: false,
            why: format!(
                "`{id}`({})이 아직 돌고 있습니다. {}초째입니다.",
                snap.label,
                snap.elapsed_ms / 1000
            ),
            next: format!("같은 인자로 `wait.until`을 다시 부르세요 — `job: \"{id}\"`."),
            elapsed_ms: at.elapsed().as_millis() as u64,
            tail: self.jobs.tail(id, TAIL_BYTES),
            exit_code: None,
        })
    }

    /// 명령을 되물어 참이 될 때까지. **원격 빌드·CI·파일 생성·포트 열림이 전부 이 갈래다.**
    ///
    /// 참/거짓은 기본이 종료 코드 0이고, `matches`를 주면 출력이 그 정규식에 걸릴 때 참이다.
    async fn until_probe(
        &self,
        command: &str,
        matches: Option<&str>,
        gap: std::time::Duration,
        budget: std::time::Duration,
        at: std::time::Instant,
    ) -> zyris::Result<Outcome> {
        let re = matches
            .map(|p| {
                regex::Regex::new(p).map_err(|e| {
                    WireError::invalid_params(format!("matches가 정규식이 아닙니다: {e}"))
                })
            })
            .transpose()?;
        let root = self.jobs.root();
        let mut last;
        let mut rounds = 0u32;
        loop {
            // **적어도 한 번은 물어본다.** 예산이 짧다고 한 번도 안 보고 돌아오면
            // 부른 쪽은 아무것도 얻지 못한다.
            rounds += 1;
            let left = budget.saturating_sub(at.elapsed());
            let limit = PROBE_LIMIT.min(left.max(std::time::Duration::from_secs(1)));
            let (ok, out) = probe_once(command, root, limit).await;
            last = out;
            let hit = match &re {
                Some(re) => re.is_match(&last),
                None => ok,
            };
            if hit {
                return Ok(Outcome {
                    done: true,
                    why: format!("{rounds}번째 확인에서 조건이 참이 되었습니다."),
                    next: "끝났습니다. 다음 일을 하세요.".into(),
                    elapsed_ms: at.elapsed().as_millis() as u64,
                    tail: tail_of(&last),
                    exit_code: None,
                });
            }
            // 다음 회차를 돌릴 시간이 없으면 여기서 답한다.
            if budget.saturating_sub(at.elapsed()) <= gap {
                break;
            }
            tokio::time::sleep(gap).await;
        }
        Ok(Outcome {
            done: false,
            why: format!("{rounds}번 확인했지만 아직 조건이 참이 아닙니다."),
            next: "같은 인자로 `wait.until`을 다시 부르세요.".into(),
            elapsed_ms: at.elapsed().as_millis() as u64,
            tail: tail_of(&last),
            exit_code: None,
        })
    }

    /// attacca work가 **사람이나 에이전트가 움직여야 하는 자리**에 닿기를 기다린다.
    ///
    /// 저쪽은 알림이 없어 폴링뿐이다. 그래도 이 갈래가 여기 있어야 하는 이유는
    /// 하나다 — **상류 zyris는 attacca를 모른다.** 셋을 한 도구로 묶을 수 있는 곳이
    /// 이 리포뿐이다.
    async fn until_work(
        &self,
        work_id: &str,
        budget: std::time::Duration,
        at: std::time::Instant,
    ) -> zyris::Result<Outcome> {
        let api = self.api()?;
        let mut state;
        loop {
            let work = api.get_work(work_id.to_string()).await?;
            state = work.state;
            if work_needs_someone(state) {
                let name = crate::tools::work::state_name(format!("{state:?}"));
                return Ok(Outcome {
                    done: true,
                    why: format!("work `{work_id}`이 `{name}`에 닿았습니다."),
                    next: format!(
                        "`work.status`로 `work_id: \"{work_id}\"`를 읽고 다음에 무엇이 \
                         필요한지 사람에게 말하세요."
                    ),
                    elapsed_ms: at.elapsed().as_millis() as u64,
                    tail: String::new(),
                    exit_code: None,
                });
            }
            if budget.saturating_sub(at.elapsed()) <= WORK_EVERY {
                break;
            }
            tokio::time::sleep(WORK_EVERY).await;
        }
        let name = crate::tools::work::state_name(format!("{state:?}"));
        Ok(Outcome {
            done: false,
            why: format!("work `{work_id}`이 아직 `{name}`입니다."),
            next: format!("같은 인자로 `wait.until`을 다시 부르세요 — `work: \"{work_id}\"`."),
            elapsed_ms: at.elapsed().as_millis() as u64,
            tail: String::new(),
            exit_code: None,
        })
    }

    fn finished(&self, id: &str, snap: Snapshot, at: std::time::Instant) -> Outcome {
        let ok = snap.exit_code == Some(0);
        Outcome {
            done: true,
            why: format!(
                "`{id}`({})이 {}초 만에 끝났습니다. 종료 코드 {}.",
                snap.label,
                snap.elapsed_ms / 1000,
                snap.exit_code.unwrap_or(-1)
            ),
            next: if ok {
                "끝났습니다. 전문이 필요하면 `wait.logs`로 가져오세요.".into()
            } else {
                format!("실패했습니다. `wait.logs`로 `job: \"{id}\"`의 출력을 읽어 원인을 보세요.")
            },
            elapsed_ms: at.elapsed().as_millis() as u64,
            tail: self.jobs.tail(id, TAIL_BYTES),
            exit_code: snap.exit_code,
        }
    }
}

fn job_ref(s: Snapshot) -> JobRef {
    JobRef {
        id: s.id,
        label: s.label,
        running: s.running,
        exit_code: s.exit_code,
        elapsed_ms: s.elapsed_ms,
    }
}

/// 되묻기 한 회차. **작업 목록에 남기지 않는다** — 되물을 때마다 줄이 쌓이면
/// `/jobs`가 못 쓰게 되고, 되묻기는 작업이 아니라 질문이다.
async fn probe_once(command: &str, root: &std::path::Path, limit: std::time::Duration) -> (bool, String) {
    let mut cmd = tokio::process::Command::new("/bin/sh");
    cmd.arg("-c").arg(command);
    cmd.current_dir(root);
    cmd.env("TERM", "dumb").env("NO_COLOR", "1");
    cmd.stdin(std::process::Stdio::null());
    #[cfg(unix)]
    cmd.process_group(0);

    match tokio::time::timeout(limit, cmd.output()).await {
        Ok(Ok(out)) => {
            let mut strip = crate::tools::jobs::Stripper::default();
            let mut text = strip.push(&out.stdout);
            text.push_str(&strip.push(&out.stderr));
            text.push_str(&strip.flush());
            (out.status.success(), text)
        }
        // **시간이 넘거나 못 띄우면 그 회차는 거짓이다.** 오류로 만들지 않는다 —
        // 아직 안 떠 있는 서버를 기다리는 것이 이 도구의 정상 쓰임이다.
        Ok(Err(e)) => (false, format!("되묻기를 띄우지 못했습니다: {e}")),
        Err(_) => (false, format!("되묻기가 {}초를 넘겨 끊었습니다.", limit.as_secs())),
    }
}

fn tail_of(text: &str) -> String {
    let from = text.floor_char_boundary(text.len().saturating_sub(TAIL_BYTES));
    text[from..].to_string()
}

fn empty_chunk() -> Chunk {
    Chunk { text: String::new(), next_offset: 0, more: false, dropped: 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn waits() -> Waits {
        let (tx, rx) = watch::channel(None);
        // 보내는 쪽을 살려 둔다 — 떨어뜨리면 `borrow`가 마지막 값을 보긴 하지만
        // 테스트가 무엇을 재는지 흐려진다.
        std::mem::forget(tx);
        Waits::new(Jobs::new(std::env::temp_dir()), rx)
    }

    async fn wait_for(w: &Waits, id: &str) {
        let mut ended = w.jobs.ended(id).expect("작업이 있어야 한다");
        while !*ended.borrow() {
            ended.changed().await.expect("보내는 쪽이 살아 있어야 한다");
        }
    }

    #[tokio::test]
    async fn starting_a_job_says_what_to_do_next() {
        let w = waits();
        let started = w.start(Some("echo hi".into()), None, None, None, None).await.unwrap();
        assert_eq!(started.id, "b1");
        assert_eq!(started.label, "echo hi");
        // **다음에 무엇을 할지 말해 준다.** 없으면 에이전트가 걸어 놓고 잊는다.
        assert!(started.next.contains("wait.until"), "{}", started.next);
    }

    #[tokio::test]
    async fn a_job_shows_up_in_the_list_and_its_logs_are_readable() {
        let w = waits();
        let id = w.start(Some("echo 안녕".into()), None, None, None, None).await.unwrap().id;
        wait_for(&w, &id).await;

        let list = w.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert!(!list[0].running);

        let logs = w.logs(id.clone(), None).await.unwrap();
        assert!(logs.text.contains("안녕"), "{}", logs.text);
        assert_eq!(logs.exit_code, Some(0));
        assert!(!logs.more);
        // 이어 읽으면 빈 것이 온다 — 같은 것을 두 번 주면 안 된다.
        assert_eq!(w.logs(id, Some(logs.next_offset)).await.unwrap().text, "");
    }

    /// 잘렸으면 **잘렸다고 말하고**, 이어 읽을 자리를 준다.
    #[tokio::test]
    async fn a_long_log_is_cut_but_says_there_is_more() {
        let w = waits();
        let id = w
            .start(Some("head -c 40000 /dev/zero | tr '\\0' 'a'".into()), None, None, None, None)
            .await
            .unwrap()
            .id;
        wait_for(&w, &id).await;
        let first = w.logs(id.clone(), None).await.unwrap();
        assert!(first.more);
        assert_eq!(first.text.len(), LOGS_BYTES);
        let second = w.logs(id, Some(first.next_offset)).await.unwrap();
        assert!(!second.text.is_empty());
    }

    #[tokio::test]
    async fn stopping_a_job_that_already_ended_is_not_an_error() {
        let w = waits();
        let id = w.start(Some("true".into()), None, None, None, None).await.unwrap().id;
        wait_for(&w, &id).await;
        assert!(w.stop(id).await.is_ok());
    }

    /// 조용히 빈 결과를 주면 도구가 고장 난 줄 알고 다른 길로 같은 일을 시도한다.
    #[tokio::test]
    async fn an_unknown_job_is_an_error_not_an_empty_answer() {
        let w = waits();
        assert!(w.logs("b9".into(), None).await.is_err());
        assert!(w.stop("b9".into()).await.is_err());
    }

    /// 인자가 틀린 것은 오류다. **그것만이 오류다.**
    #[tokio::test]
    async fn starting_with_neither_command_nor_argv_is_an_error() {
        let w = waits();
        assert!(w.start(None, None, None, None, None).await.is_err());
    }

    /// **예산은 마감을 넘지 않는다.** attacca가 60초에 오류로 끊으므로 그 전에 우리가
    /// 성공으로 답해야 한다. 순수 함수로 재는 이유는 환경 변수를 흔들면 같이 도는
    /// 다른 테스트를 밟기 때문이다.
    #[test]
    fn the_budget_never_outlives_the_wire_deadline() {
        let deadline = Some(std::time::Duration::from_secs(55));
        // 10분을 달라고 해도 마감 안쪽으로 자른다.
        assert_eq!(budget(deadline, Some(600_000)), std::time::Duration::from_secs(50));
        // 짧게 잡았으면 그쪽이 이긴다 — **마감은 상한이지 기본값이 아니다.**
        assert_eq!(budget(deadline, Some(3_000)), std::time::Duration::from_millis(3_000));
        assert_eq!(budget(deadline, None), std::time::Duration::from_secs(50));
        // 마감이 꺼져 있으면(저쪽이 고쳐지면) 에이전트가 준 값만 본다.
        assert_eq!(budget(None, Some(600_000)), std::time::Duration::from_secs(600));
    }

    /// **지금 버그의 정본이다.** 안 끝난 것은 실패가 아니다 — `exec`이 주는
    /// `timed_out: true, exit_code: -1`을 에이전트가 실패로 읽는 것이 이 작업의 이유다.
    #[tokio::test]
    async fn an_unfinished_wait_answers_success_not_an_error() {
        let w = waits();
        let id = w.start(Some("sleep 30".into()), None, None, None, None).await.unwrap().id;
        let out = w
            .until(Some(id.clone()), None, None, None, None, Some(1500))
            .await
            .expect("안 끝났다고 오류를 내면 안 된다");
        assert!(!out.done);
        assert_eq!(out.exit_code, None);
        // 다시 부르라고 말해야 한다. 안 그러면 에이전트가 "멈췄다"로 읽고 포기한다.
        assert!(out.next.contains("wait.until"), "{}", out.next);
        assert!(out.why.contains(&id), "{}", out.why);
        w.stop(id).await.unwrap();
    }

    /// 마감 안쪽에 돌아온다.
    #[tokio::test]
    async fn a_wait_answers_before_its_budget_runs_out() {
        let w = waits();
        let id = w.start(Some("sleep 30".into()), None, None, None, None).await.unwrap().id;
        let at = std::time::Instant::now();
        let _ = w.until(Some(id.clone()), None, None, None, None, Some(1000)).await.unwrap();
        assert!(at.elapsed() < std::time::Duration::from_secs(3), "{:?}", at.elapsed());
        w.stop(id).await.unwrap();
    }

    /// 끝나면 기다리지 않고 그 자리에서 돌아온다.
    #[tokio::test]
    async fn a_wait_returns_the_moment_the_job_ends() {
        let w = waits();
        let id = w.start(Some("sleep 1; echo 끝".into()), None, None, None, None).await.unwrap().id;
        let at = std::time::Instant::now();
        let out = w.until(Some(id), None, None, None, None, Some(20_000)).await.unwrap();
        assert!(out.done, "{}", out.why);
        assert_eq!(out.exit_code, Some(0));
        assert!(out.tail.contains("끝"), "{}", out.tail);
        assert!(at.elapsed() < std::time::Duration::from_secs(5), "{:?}", at.elapsed());
    }

    /// 이미 끝난 것을 기다리라고 하면 곧바로 끝났다고 답한다.
    #[tokio::test]
    async fn waiting_on_an_already_finished_job_answers_at_once() {
        let w = waits();
        let id = w.start(Some("exit 7".into()), None, None, None, None).await.unwrap().id;
        wait_for(&w, &id).await;
        let out = w.until(Some(id), None, None, None, None, Some(20_000)).await.unwrap();
        assert!(out.done);
        assert_eq!(out.exit_code, Some(7));
        // 실패한 작업은 **어디를 보라고** 말한다.
        assert!(out.next.contains("wait.logs"), "{}", out.next);
    }

    /// 무엇을 기다릴지 정확히 하나여야 한다.
    #[tokio::test]
    async fn until_needs_exactly_one_thing_to_wait_for() {
        let w = waits();
        assert!(w.until(None, None, None, None, None, None).await.is_err());
        assert!(w
            .until(Some("b1".into()), Some("true".into()), None, None, None, None)
            .await
            .is_err());
    }

    /// 참이 되면 그 자리에서 끝난다.
    #[tokio::test]
    async fn a_probe_that_succeeds_ends_the_wait() {
        let w = waits();
        let out = w.until(None, Some("true".into()), None, None, None, Some(20_000)).await.unwrap();
        assert!(out.done, "{}", out.why);
    }

    /// 출력이 패턴에 걸리면 참이다 — **원격 빌드 상태를 이렇게 본다.**
    /// 종료 코드가 0이어도 패턴이 우선이다.
    #[tokio::test]
    async fn a_probe_can_match_on_output_instead_of_exit_code() {
        let w = waits();
        let out = w
            .until(
                None,
                Some("echo status=queued".into()),
                None,
                Some("completed".into()),
                None,
                Some(1500),
            )
            .await
            .unwrap();
        assert!(!out.done, "{}", out.why);
        assert!(out.tail.contains("queued"), "{}", out.tail);
    }

    /// **되물을 명령은 싸야 한다.** 간격을 바닥 밑으로 내려 달라고 해도 안 내려간다.
    #[test]
    fn a_probe_command_cannot_run_more_often_than_the_floor() {
        assert_eq!(probe_gap(Some(10)), std::time::Duration::from_millis(PROBE_FLOOR_MS));
        assert_eq!(probe_gap(None), std::time::Duration::from_millis(PROBE_EVERY_MS));
        assert_eq!(probe_gap(Some(30_000)), std::time::Duration::from_millis(30_000));
    }

    /// **실제로 다시 돌린다.** 두 번째 회차에서야 참이 되는 명령으로 잰다.
    #[tokio::test]
    async fn a_probe_actually_runs_again_until_it_is_true() {
        let w = waits();
        let dir = tempfile::tempdir().unwrap();
        let n = dir.path().join("n").display().to_string();
        let out = w
            .until(
                None,
                Some(format!("echo x >> {n}; test $(wc -c < {n}) -ge 3")),
                None,
                None,
                Some(0), // 바닥(2초)으로 올라간다
                Some(20_000),
            )
            .await
            .unwrap();
        assert!(out.done, "{}", out.why);
        // 한 회차에 2바이트씩 쌓이므로 4바이트면 두 번 돌았다는 뜻이다.
        assert_eq!(std::fs::read(&n).unwrap().len(), 4);
    }

    /// 안 걸려도 마감 안에 **성공으로** 돌아오고 다시 부르라고 말한다.
    #[tokio::test]
    async fn a_probe_that_never_succeeds_still_answers_success() {
        let w = waits();
        let at = std::time::Instant::now();
        let out = w.until(None, Some("false".into()), None, None, None, Some(1500)).await.unwrap();
        assert!(!out.done);
        assert!(out.next.contains("wait.until"), "{}", out.next);
        assert!(at.elapsed() < std::time::Duration::from_secs(4), "{:?}", at.elapsed());
    }

    /// 정규식이 아니면 인자 오류다. 이것은 진짜 오류다.
    #[tokio::test]
    async fn a_broken_pattern_is_an_argument_error() {
        let w = waits();
        assert!(w
            .until(None, Some("true".into()), None, Some("[".into()), None, Some(1000))
            .await
            .is_err());
    }

    /// **관문·멈춤·끝에서만 깨운다.** 저쪽이 도는 중에 돌려보내면 에이전트는 할 일이
    /// 없는데 깨어나고, 그러면 확인만 되풀이한다.
    #[test]
    fn a_work_only_wakes_the_agent_where_someone_must_move() {
        use zyris_attacca::ZWorkState::*;
        for state in [CheckingRequirements, Planning, Executing, Verifying] {
            assert!(!work_needs_someone(state), "{state:?}");
        }
        for state in
            [Draft, AwaitingGoalApproval, AwaitingPlanApproval, Halted, Done, Failed, Cancelled]
        {
            assert!(work_needs_someone(state), "{state:?}");
        }
    }

    /// 안 붙었으면 그렇게 말한다. **조용히 실패하면 원인을 못 찾는다.**
    #[tokio::test]
    async fn waiting_on_a_work_before_attacca_is_up_says_so() {
        let w = waits();
        let err = w.until(None, None, Some("w_1".into()), None, None, Some(1000)).await;
        assert!(err.is_err());
    }

    /// `KEY=VALUE` 줄이 그대로 환경이 된다.
    #[tokio::test]
    async fn env_lines_reach_the_command() {
        let w = waits();
        let id = w
            .start(
                Some("echo $FOO".into()),
                None,
                None,
                Some(vec!["FOO=바".into()]),
                Some("환경 시험".into()),
            )
            .await
            .unwrap()
            .id;
        wait_for(&w, &id).await;
        assert_eq!(w.logs(id, None).await.unwrap().text.trim(), "바");
    }
}
