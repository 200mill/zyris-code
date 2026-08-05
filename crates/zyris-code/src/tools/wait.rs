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
use zyris_attacca::AttaccaApiClient;

use crate::tools::jobs::{Chunk, Jobs, Snapshot, Spec};

/// 한 번의 `logs`가 주는 최대 크기. 남은 것은 `more`로 말한다.
const LOGS_BYTES: usize = 16_000;

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

    /// Kill a background job and everything it started. A job that already ended is fine.
    async fn stop(&self, job: String) -> zyris::Result<()>;
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

    #[allow(dead_code)] // `until`의 work 갈래가 쓴다.
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

    async fn stop(&self, job: String) -> zyris::Result<()> {
        self.known(&job)?;
        self.jobs.stop(&job);
        Ok(())
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
