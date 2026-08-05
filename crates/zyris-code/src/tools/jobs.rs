//! 배경에 건 명령. **띄우고, 모으고, 그룹째 죽인다.**
//!
//! 여기는 프로세스와 버퍼만 안다. 와이어 표면(마감 계약·`until`의 갈래)은 `wait.rs`다.
//! 둘을 가른 이유는 테스트다 — 한 파일이면 마감을 재는 테스트마저 진짜 프로세스를
//! 띄워야 한다.

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use regex::Regex;
use tokio::io::AsyncReadExt;
use tokio::sync::watch;

/// 한 작업의 출력 상한. 넘치면 앞이 사라지고 `dropped`가 그만큼 는다.
const RING_CAP: usize = 1024 * 1024;
/// 들고 있는 작업 수. 넘치면 **끝난 것 중** 오래된 것부터 버린다.
const KEEP: usize = 20;
/// `stop` 뒤 SIGKILL까지의 유예. 배포 스크립트가 정리할 틈이다.
const GRACE: Duration = Duration::from_secs(2);
/// 앱을 끌 때의 유예. **여기서는 짧아야 한다** — 나가는 길을 2초 붙들면 안 눌린 줄 안다.
const QUIT_GRACE: Duration = Duration::from_millis(300);

/// ANSI 이스케이프. CSI(`ESC [ … 최종문자`)·OSC(`ESC ] … BEL|ST`)·그 밖의 두 글자짜리.
///
/// `\x1b[`는 두 글자짜리 갈래에서 **일부러 뺐다**(`[` = 0x5B가 `[@-Z\\-_]`에 없다) —
/// 안 그러면 아직 안 끝난 CSI의 머리를 두 글자로 먹어 버린다.
static ESCAPES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[@-Z\\-_]")
        .expect("정적 정규식")
});

/// 터미널 출력을 **에이전트가 읽을 수 있는 글자로** 바꾼다.
///
/// zyris-caps의 주석이 이유를 적어 두었다: *"a tool result carrying a raw `U+001B` is
/// rejected outright by at least one agent runtime."* cargo 출력에는 색이 붙는다.
/// 상류의 `strip_controls`는 `pub(crate)`라 여기서 쓸 수 없다.
///
/// 셋을 한다: 이스케이프를 벗기고, 캐리지 리턴이 다시 쓰는 줄을 접고, `\n`·`\t` 밖의
/// C0를 지운다. **청크 경계에서 잘린 것은 다음 청크까지 들고 있는다** — 문자도,
/// 이스케이프도, 줄 끝의 `\r`도.
#[derive(Debug, Default)]
pub struct Stripper {
    /// 아직 문자로 못 읽은 바이트, 또는 안 끝난 이스케이프.
    carry: Vec<u8>,
    /// 아직 `\n`을 못 만난 줄. 캐리지 리턴이 오면 이걸 버린다.
    line: String,
    /// 앞 청크가 `\r`로 끝났다. **다음 글자가 `\n`이면 CRLF라 줄을 버리면 안 된다.**
    pending_cr: bool,
}

impl Stripper {
    /// 새로 온 바이트를 넣고, 지금 확정된 글자를 준다. 줄이 끝나야 확정된다.
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.carry.extend_from_slice(bytes);
        // 문자로 못 읽는 꼬리는 다음 청크까지 들고 있는다.
        let valid = match std::str::from_utf8(&self.carry) {
            Ok(_) => self.carry.len(),
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            // 진짜 깨진 바이트는 버린다. 들고 있어 봐야 영영 안 읽힌다.
            Err(e) => e.valid_up_to() + e.error_len().unwrap_or(1),
        };
        let text = String::from_utf8_lossy(&self.carry[..valid]).into_owned();
        self.carry.drain(..valid);

        // 안 끝난 이스케이프는 되돌려 놓는다 — 다음 청크와 이어야 벗겨진다.
        let (ready, held) = split_incomplete_escape(&text);
        if !held.is_empty() {
            let mut back = held.as_bytes().to_vec();
            back.extend_from_slice(&self.carry);
            self.carry = back;
        }

        let stripped = ESCAPES.replace_all(ready, "");
        self.feed(&stripped)
    }

    /// 프로세스가 끝났을 때 남은 것을 마저 낸다.
    ///
    /// **끝에 걸린 `\r`은 줄을 지우지 않는다.** 터미널에서도 마지막 진행 표시는
    /// 화면에 남아 있고, 사람이 마지막으로 본 것이 그것이다.
    pub fn flush(&mut self) -> String {
        let rest = String::from_utf8_lossy(&std::mem::take(&mut self.carry)).into_owned();
        self.pending_cr = false;
        let stripped = ESCAPES.replace_all(&rest, "").into_owned();
        let mut out = self.feed(&stripped);
        self.pending_cr = false;
        out.push_str(&std::mem::take(&mut self.line));
        out
    }

    /// 줄 단위로 확정한다. **`\r`은 그 줄을 다시 쓰는 것**이라 앞의 것을 버린다.
    fn feed(&mut self, text: &str) -> String {
        let mut out = String::new();
        let mut chars = text.chars().peekable();
        // 앞 청크가 `\r`로 끝났다. 이 청크의 첫 글자가 `\n`이면 CRLF이므로 줄은 살린다.
        if std::mem::take(&mut self.pending_cr) && chars.peek() != Some(&'\n') {
            self.line.clear();
        }
        while let Some(ch) = chars.next() {
            match ch {
                '\n' => {
                    out.push_str(&std::mem::take(&mut self.line));
                    out.push('\n');
                }
                // CRLF는 그냥 줄바꿈이다. 여기서 줄을 버리면 윈도우 출력이 통째로 사라진다.
                '\r' => match chars.peek() {
                    Some('\n') => {}
                    Some(_) => self.line.clear(),
                    None => self.pending_cr = true,
                },
                '\t' => self.line.push('\t'),
                c if (c as u32) < 0x20 || c as u32 == 0x7f => {}
                c => self.line.push(c),
            }
        }
        out
    }
}

/// 배경에 걸 것.
#[derive(Debug, Clone, Default)]
pub struct Spec {
    /// 셸 한 줄. `argv`와 **정확히 하나만** 준다.
    pub command: Option<String>,
    /// 셸 없이 그대로 띄울 프로그램과 인자.
    pub argv: Option<Vec<String>>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub label: Option<String>,
}

/// 사람과 에이전트가 보는 한 줄.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub id: String,
    pub label: String,
    pub running: bool,
    pub exit_code: Option<i32>,
    pub elapsed_ms: u64,
}

struct Job {
    label: String,
    started: Instant,
    pid: Option<u32>,
    exit_code: Option<i32>,
    running: bool,
    out: Ring,
    /// 끝났음을 기다리는 쪽에게 알린다. `wait.until`이 이것을 기다린다.
    ended: watch::Sender<bool>,
}

/// 이 앱이 배경에 걸어 둔 것 전부.
///
/// **레지스트리는 하나다.** 도구·화면·`/jobs`가 같은 것을 봐야 한다 — 두 벌이 되면
/// 한쪽만 고쳤을 때 조용히 갈라진다. `Bridge::set_undo`와 같은 배선이다.
#[derive(Clone)]
pub struct Jobs {
    root: PathBuf,
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    jobs: Vec<(String, Job)>,
    next: u64,
}

impl Jobs {
    pub fn new(root: PathBuf) -> Jobs {
        Jobs { root, inner: Arc::new(Mutex::new(Inner::default())) }
    }

    /// 상대경로가 풀리는 자리. 되묻기도 여기서 돈다 — **작업과 질문이 같은 곳을 봐야
    /// 한다.** 다르면 `ls`의 결과가 도구마다 달라진다.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 배경에 걸고 즉시 돌아온다. **실행을 기다리지 않는다.**
    pub fn start(&self, spec: Spec) -> Result<String, String> {
        let mut cmd = build(&spec, &self.root)?;
        let label = spec
            .label
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| spec.command.clone())
            .or_else(|| spec.argv.as_ref().map(|a| a.join(" ")))
            .unwrap_or_default();

        let mut child = cmd.spawn().map_err(|e| format!("띄우지 못했습니다: {e}"))?;
        let pid = child.id();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (ended, _) = watch::channel(false);

        let id = {
            let mut inner = self.inner.lock().unwrap();
            inner.next += 1;
            let id = format!("b{}", inner.next);
            inner.jobs.push((
                id.clone(),
                Job {
                    label,
                    started: Instant::now(),
                    pid,
                    exit_code: None,
                    running: true,
                    out: Ring::new(RING_CAP),
                    ended,
                },
            ));
            // **끝난 것부터 버린다.** 도는 것을 버리면 죽일 손잡이를 잃는다.
            while inner.jobs.len() > KEEP {
                let Some(at) = inner.jobs.iter().position(|(_, j)| !j.running) else { break };
                inner.jobs.remove(at);
            }
            id
        };

        let this = self.clone();
        let for_task = id.clone();
        tokio::spawn(async move {
            // stdout·stderr를 **한 버퍼로** 모은다. 순서가 곧 읽는 사람의 이해다.
            let pump = {
                let (a, b) = (this.clone(), this.clone());
                let (ia, ib) = (for_task.clone(), for_task.clone());
                async move {
                    tokio::join!(drain(a, ia, stdout), drain(b, ib, stderr));
                }
            };
            let (status, ()) = tokio::join!(child.wait(), pump);
            let code = status.ok().and_then(|s| s.code()).unwrap_or(-1);
            let mut inner = this.inner.lock().unwrap();
            if let Some((_, job)) = inner.jobs.iter_mut().find(|(k, _)| *k == for_task) {
                job.running = false;
                job.exit_code = Some(code);
                let _ = job.ended.send(true);
            }
        });
        Ok(id)
    }

    pub fn snapshot(&self, id: &str) -> Option<Snapshot> {
        let inner = self.inner.lock().unwrap();
        inner.jobs.iter().find(|(k, _)| k == id).map(|(k, j)| snap(k, j))
    }

    pub fn list(&self) -> Vec<Snapshot> {
        let inner = self.inner.lock().unwrap();
        inner.jobs.iter().map(|(k, j)| snap(k, j)).collect()
    }

    pub fn read(&self, id: &str, offset: u64) -> Option<Chunk> {
        let inner = self.inner.lock().unwrap();
        inner.jobs.iter().find(|(k, _)| k == id).map(|(_, j)| j.out.read(offset))
    }

    pub fn tail(&self, id: &str, bytes: usize) -> String {
        let inner = self.inner.lock().unwrap();
        inner
            .jobs
            .iter()
            .find(|(k, _)| k == id)
            .map(|(_, j)| j.out.tail(bytes))
            .unwrap_or_default()
    }

    /// 끝났음을 기다릴 손잡이. **이미 끝났으면 처음부터 참이다.**
    pub fn ended(&self, id: &str) -> Option<watch::Receiver<bool>> {
        let inner = self.inner.lock().unwrap();
        inner.jobs.iter().find(|(k, _)| k == id).map(|(_, j)| j.ended.subscribe())
    }

    /// 도는 작업을 그룹째 죽인다. 이미 끝났으면 거짓이다 — 오류는 아니다.
    pub fn stop(&self, id: &str) -> bool {
        let Some(pid) = self.running_pid(id) else { return false };
        if !term_tree(pid) {
            return true;
        }
        // 유예를 준 뒤 확실히 끝낸다. **나가는 길을 붙들지 않으려고** 따로 돈다.
        std::thread::spawn(move || {
            std::thread::sleep(GRACE);
            kill_tree(pid);
        });
        true
    }

    /// 앱을 끄기 전에 부른다. **고아를 남기지 않는다.**
    ///
    /// 여기서는 유예가 짧다(`QUIT_GRACE`). 끝나기를 2초씩 기다리면 사람은 앱이 안
    /// 닫힌 줄 안다. 그래도 TERM을 먼저 보내는 이유는 정리할 틈을 주기 위해서다.
    pub fn stop_all(&self) {
        let pids: Vec<u32> = {
            let inner = self.inner.lock().unwrap();
            inner.jobs.iter().filter(|(_, j)| j.running).filter_map(|(_, j)| j.pid).collect()
        };
        if pids.is_empty() {
            return;
        }
        for pid in &pids {
            term_tree(*pid);
        }
        std::thread::sleep(QUIT_GRACE);
        for pid in &pids {
            kill_tree(*pid);
        }
    }

    fn running_pid(&self, id: &str) -> Option<u32> {
        let inner = self.inner.lock().unwrap();
        match inner.jobs.iter().find(|(k, _)| k == id) {
            Some((_, j)) if j.running => j.pid,
            _ => None,
        }
    }

    fn absorb(&self, id: &str, text: &str) {
        let mut inner = self.inner.lock().unwrap();
        if let Some((_, job)) = inner.jobs.iter_mut().find(|(k, _)| k == id) {
            job.out.push(text);
        }
    }
}

fn snap(id: &str, j: &Job) -> Snapshot {
    Snapshot {
        id: id.to_string(),
        label: j.label.clone(),
        running: j.running,
        exit_code: j.exit_code,
        elapsed_ms: j.started.elapsed().as_millis() as u64,
    }
}

/// 한 스트림을 끝까지 읽어 링에 넣는다.
async fn drain<R: tokio::io::AsyncRead + Unpin>(jobs: Jobs, id: String, stream: Option<R>) {
    let Some(mut stream) = stream else { return };
    let mut strip = Stripper::default();
    let mut buf = [0u8; 8192];
    loop {
        match stream.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let text = strip.push(&buf[..n]);
                if !text.is_empty() {
                    jobs.absorb(&id, &text);
                }
            }
        }
    }
    let rest = strip.flush();
    if !rest.is_empty() {
        jobs.absorb(&id, &rest);
    }
}

/// 명령을 만든다. **색이 덜 나오게** 환경을 미리 깔고, 사람이 준 것이 이긴다.
fn build(spec: &Spec, root: &Path) -> Result<tokio::process::Command, String> {
    let mut cmd = match (&spec.command, &spec.argv) {
        (Some(_), Some(_)) | (None, None) => {
            return Err("command와 argv 중 정확히 하나를 주세요.".into())
        }
        (Some(line), None) => {
            if line.trim().is_empty() {
                return Err("command가 비어 있습니다.".into());
            }
            let mut c = tokio::process::Command::new("/bin/sh");
            c.arg("-c").arg(line);
            c
        }
        (None, Some(argv)) => {
            let Some((program, rest)) = argv.split_first() else {
                return Err("argv가 비어 있습니다.".into());
            };
            let mut c = tokio::process::Command::new(program);
            c.args(rest);
            c
        }
    };
    cmd.current_dir(spec.cwd.clone().unwrap_or_else(|| root.to_path_buf()));
    cmd.env("TERM", "dumb").env("NO_COLOR", "1").env("CARGO_TERM_COLOR", "never");
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(unix)]
    {
        // **새 그룹의 리더로 만든다.** 그래야 셸이 남긴 손자까지 한 번에 죽는다.
        // 빼고 재 보면 `stopping_a_job_kills_the_whole_process_tree`가 문다 — 실제로
        // 빼서 확인해 뒀다. tokio의 `Command`가 이것을 직접 준다(`CommandExt` 불필요).
        cmd.process_group(0);
    }
    Ok(cmd)
}

/// 프로세스 그룹에 SIGTERM. 보냈으면 참이다.
///
/// **setpgid 경합을 넘긴다** — 자식이 아직 그룹을 만들기 전이면 신호가 실패한다.
/// capkit의 `kill_tree`가 하는 것과 같은 재시도이고, 없으면 방금 건 작업을 곧바로
/// 멈출 때 조용히 안 죽는다.
#[cfg(unix)]
fn term_tree(pid: u32) -> bool {
    let group = -(pid as i32);
    for _ in 0..50 {
        // SAFETY: 음수 pid는 프로세스 그룹이다. 우리가 방금 만든 그룹이고 우리는 그 안에 없다.
        if unsafe { libc::kill(group, libc::SIGTERM) } == 0 {
            return true;
        }
        // 리더가 이미 죽었으면 그룹은 앞으로 안 생긴다.
        if unsafe { libc::kill(pid as i32, 0) } != 0 {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[cfg(unix)]
fn kill_tree(pid: u32) {
    // SAFETY: 위와 같다.
    unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
}

/// 윈도우: `taskkill /T`가 트리를 죽인다. 부드러운 신호가 따로 없어 한 번에 간다.
#[cfg(not(unix))]
fn term_tree(pid: u32) -> bool {
    kill_tree(pid);
    false
}

#[cfg(not(unix))]
fn kill_tree(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .output();
}

/// 한 작업의 출력. **stdout과 stderr를 섞어 담는다** — cargo는 진행을 stderr로 내므로
/// 나눠 담으면 순서가 사라지고, 순서가 곧 읽는 사람의 이해다.
#[derive(Debug)]
pub struct Ring {
    buf: Vec<u8>,
    cap: usize,
    /// 앞에서 버린 바이트 수. **절대 위치(offset)의 기준이다.**
    dropped: u64,
}

/// `Ring::read`가 주는 것.
///
/// `more`와 `dropped`를 가르는 이유는 capkit의 `PtyRead`와 같다 — 읽는 쪽에게
/// **유일하게 중요한 질문은 "다시 불러서 받을 수 있는가"**다. 하나의 "잘림" 깃발로
/// 뭉개면 그 질문에 답할 수 없다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub text: String,
    /// 다음에 이 위치를 주면 이어서 받는다.
    pub next_offset: u64,
    /// 아직 버퍼에 남은 것이 있다. **다시 부르면 받는다.**
    pub more: bool,
    /// 넘쳐서 잃은 바이트. **다시 불러도 안 돌아온다.**
    pub dropped: u64,
}

impl Ring {
    pub fn new(cap: usize) -> Ring {
        Ring { buf: Vec::new(), cap, dropped: 0 }
    }

    pub fn push(&mut self, text: &str) {
        self.buf.extend_from_slice(text.as_bytes());
        if self.buf.len() > self.cap {
            let cut = self.buf.len() - self.cap;
            self.buf.drain(..cut);
            self.dropped += cut as u64;
        }
    }

    /// 절대 위치부터 읽는다. 이미 버려진 자리를 달라고 하면 **남아 있는 앞부터** 준다.
    ///
    /// `more`는 늘 거짓이다 — 남은 것을 전부 주기 때문이다. 한 번에 다 주기에 큰 경우는
    /// 자르는 쪽(`wait.logs`)이 세운다.
    pub fn read(&self, offset: u64) -> Chunk {
        let dropped = self.dropped.saturating_sub(offset);
        let from = offset.max(self.dropped) - self.dropped;
        let from = (from as usize).min(self.buf.len());
        Chunk {
            text: String::from_utf8_lossy(&self.buf[from..]).into_owned(),
            next_offset: self.dropped + self.buf.len() as u64,
            more: false,
            dropped,
        }
    }

    /// 마지막 몇 바이트. **줄 경계에서 시작한다** — 반 토막 줄로 시작하면 읽는 사람이
    /// 무슨 일인지 알 수 없다. 통째로 들어가면 첫 줄을 버리지 않는다.
    pub fn tail(&self, bytes: usize) -> String {
        let from = self.buf.len().saturating_sub(bytes);
        let slice = &self.buf[from..];
        let start = if from == 0 {
            0
        } else {
            slice.iter().position(|b| *b == b'\n').map(|i| i + 1).unwrap_or(0)
        };
        String::from_utf8_lossy(&slice[start..]).into_owned()
    }
}

/// 꼬리가 안 끝난 이스케이프면 그 앞까지만 확정하고 나머지를 돌려준다.
fn split_incomplete_escape(text: &str) -> (&str, &str) {
    let Some(at) = text.rfind('\x1b') else { return (text, "") };
    // 이미 끝난 이스케이프면 그 자리에서 정규식이 먹는다 — 들고 있을 것이 없다.
    if ESCAPES.find_at(text, at).is_some_and(|m| m.start() == at) {
        return (text, "");
    }
    text.split_at(at)
}

#[cfg(test)]
mod jobs_tests {
    use super::*;

    fn jobs() -> Jobs {
        Jobs::new(std::env::temp_dir())
    }

    fn shell(command: &str) -> Spec {
        Spec { command: Some(command.into()), ..Default::default() }
    }

    async fn wait_for(j: &Jobs, id: &str) {
        let mut ended = j.ended(id).expect("작업이 있어야 한다");
        while !*ended.borrow() {
            ended.changed().await.expect("보내는 쪽이 살아 있어야 한다");
        }
    }

    #[tokio::test]
    async fn a_finished_job_keeps_its_output_readable() {
        let j = jobs();
        let id = j.start(shell("echo 안녕; exit 3")).unwrap();
        wait_for(&j, &id).await;
        let snap = j.snapshot(&id).unwrap();
        assert!(!snap.running);
        assert_eq!(snap.exit_code, Some(3));
        // 끝난 뒤에도 읽힌다 — `until`이 done을 준 다음 에이전트가 logs를 부른다.
        assert!(j.read(&id, 0).unwrap().text.contains("안녕"));
    }

    /// stdout과 stderr가 한 버퍼에 온다.
    #[tokio::test]
    async fn both_streams_land_in_one_buffer() {
        let j = jobs();
        let id = j.start(shell("echo 나감; echo 오류 1>&2")).unwrap();
        wait_for(&j, &id).await;
        let text = j.read(&id, 0).unwrap().text;
        assert!(text.contains("나감"), "{text}");
        assert!(text.contains("오류"), "{text}");
    }

    /// **셸만 죽이면 손자가 고아로 남는다.** 이 머신에서 그건 곧 프리즈다.
    #[tokio::test]
    async fn stopping_a_job_kills_the_whole_process_tree() {
        let j = jobs();
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("grandchild-lived");
        // 손자는 3초 뒤에 파일을 남긴다. 그룹째 죽으면 파일이 안 생긴다.
        let id = j.start(shell(&format!("(sleep 3; touch {}) & wait", marker.display()))).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(j.stop(&id));
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
        assert!(!marker.exists(), "손자가 살아남았다");
    }

    /// 앱을 끄면 도는 작업도 같이 죽는다. 고아 cargo가 이 머신에 남으면 안 된다.
    #[tokio::test]
    async fn quitting_the_app_leaves_no_running_job() {
        let j = jobs();
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("survived");
        j.start(shell(&format!("(sleep 3; touch {}) & wait", marker.display()))).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        j.stop_all();
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
        assert!(!marker.exists(), "앱이 꺼졌는데 작업이 살아남았다");
    }

    /// 이미 끝난 것을 멈추라고 하면 거짓이다. 오류는 아니다.
    #[tokio::test]
    async fn stopping_a_finished_job_is_false_not_an_error() {
        let j = jobs();
        let id = j.start(shell("true")).unwrap();
        wait_for(&j, &id).await;
        assert!(!j.stop(&id));
    }

    /// 인자는 정확히 하나여야 한다.
    #[tokio::test]
    async fn a_spec_needs_exactly_one_of_command_or_argv() {
        let j = jobs();
        assert!(j.start(Spec::default()).is_err());
        assert!(j
            .start(Spec {
                command: Some("ls".into()),
                argv: Some(vec!["ls".into()]),
                ..Default::default()
            })
            .is_err());
        assert!(j.start(Spec { command: Some("   ".into()), ..Default::default() }).is_err());
    }

    /// id는 짧아야 한다 — 에이전트가 확인할 때마다 실어 나르는 글자다.
    #[tokio::test]
    async fn ids_are_short_and_ordered() {
        let j = jobs();
        assert_eq!(j.start(shell("true")).unwrap(), "b1");
        assert_eq!(j.start(shell("true")).unwrap(), "b2");
        assert_eq!(j.list().len(), 2);
    }

    /// 색이 애초에 덜 나오게 띄운다. 사람이 준 env가 이긴다.
    #[tokio::test]
    async fn the_environment_discourages_colour_but_the_caller_wins() {
        let j = jobs();
        let id = j
            .start(Spec {
                command: Some("echo $NO_COLOR-$TERM".into()),
                env: vec![("TERM".into(), "xterm".into())],
                ..Default::default()
            })
            .unwrap();
        wait_for(&j, &id).await;
        assert_eq!(j.read(&id, 0).unwrap().text.trim(), "1-xterm");
    }
}

#[cfg(test)]
mod ring_tests {
    use super::*;

    #[test]
    fn reading_from_the_start_gives_everything_once() {
        let mut r = Ring::new(64);
        r.push("hello\n");
        let c = r.read(0);
        assert_eq!(c.text, "hello\n");
        assert_eq!(c.next_offset, 6);
        assert_eq!(c.dropped, 0);
        // 이어 읽으면 빈 것이 온다 — 같은 것을 두 번 주면 안 된다.
        assert_eq!(r.read(c.next_offset).text, "");
    }

    /// 넘치면 앞이 사라지고 **사라진 양을 말한다.** 다시 불러도 안 돌아온다.
    #[test]
    fn overflow_drops_the_front_and_says_how_much() {
        let mut r = Ring::new(8);
        r.push("0123456789");
        let c = r.read(0);
        assert_eq!(c.text, "23456789");
        assert_eq!(c.dropped, 2);
        assert_eq!(c.next_offset, 10);
    }

    /// 이미 읽은 자리를 다시 달라고 하면 잃은 것은 없다.
    #[test]
    fn reading_on_from_a_live_offset_loses_nothing() {
        let mut r = Ring::new(8);
        r.push("01234567");
        let first = r.read(0);
        r.push("89");
        let next = r.read(first.next_offset);
        assert_eq!(next.text, "89");
        assert_eq!(next.dropped, 0);
    }

    /// 꼬리는 마지막 몇 바이트다. **줄 가운데서 자르지 않는다** — 반 토막 줄은
    /// 읽는 사람을 헷갈리게 한다.
    #[test]
    fn the_tail_starts_at_a_line_boundary() {
        let mut r = Ring::new(1024);
        r.push("첫 줄\n둘째 줄\n셋째 줄\n");
        let tail = r.tail(20);
        assert!(tail.starts_with("둘째") || tail.starts_with("셋째"), "{tail}");
        assert!(tail.ends_with('\n'));
    }

    /// 통째로 들어가면 앞부터 다 준다 — 줄 경계를 찾겠다고 첫 줄을 버리면 안 된다.
    #[test]
    fn a_tail_that_covers_everything_keeps_the_first_line() {
        let mut r = Ring::new(1024);
        r.push("하나\n둘\n");
        assert_eq!(r.tail(1024), "하나\n둘\n");
    }
}

#[cfg(test)]
mod strip_tests {
    use super::*;

    /// 색 코드는 에이전트에게 도달하면 안 된다 — 런타임 하나가 통째로 거절한다.
    #[test]
    fn control_sequences_never_reach_the_agent() {
        let mut s = Stripper::default();
        let out = s.push(b"\x1b[32m   Compiling\x1b[0m zyris-code\n");
        assert_eq!(out, "   Compiling zyris-code\n");
        assert!(!out.contains('\x1b'));
    }

    /// 청크 경계에서 잘린 이스케이프를 다음 청크와 이어 붙여야 한다.
    #[test]
    fn an_escape_split_across_chunks_is_still_stripped() {
        let mut s = Stripper::default();
        let a = s.push(b"ok\x1b[3");
        let b = s.push(b"2mgreen\n");
        assert_eq!(format!("{a}{b}"), "okgreen\n");
    }

    /// 여러 바이트 문자가 청크 경계에서 잘려도 깨지지 않는다.
    #[test]
    fn a_character_split_across_chunks_survives() {
        let mut s = Stripper::default();
        let bytes = "한글".as_bytes();
        let a = s.push(&bytes[..4]);
        let b = s.push(&bytes[4..]);
        let c = s.flush();
        assert_eq!(format!("{a}{b}{c}"), "한글");
    }

    /// 캐리지 리턴은 그 줄을 다시 쓰는 것이다. 진행 표시가 수천 줄이 되면 안 된다.
    #[test]
    fn a_progress_line_is_rewritten_not_appended() {
        let mut s = Stripper::default();
        let mut out = String::new();
        out.push_str(&s.push(b"Building [=>   ] 10%\r"));
        out.push_str(&s.push(b"Building [====>] 99%\r"));
        out.push_str(&s.push(b"Building [=====] 100%\n"));
        assert_eq!(out, "Building [=====] 100%\n");
    }

    /// **CRLF는 그냥 줄바꿈이다.** `\r`을 줄 지우기로만 읽으면 윈도우 출력이 사라진다.
    #[test]
    fn a_crlf_is_a_newline_not_an_erase() {
        let mut s = Stripper::default();
        assert_eq!(s.push(b"first\r\nsecond\r\n"), "first\nsecond\n");
        // 청크가 그 사이에서 갈려도 같아야 한다.
        let mut s = Stripper::default();
        let a = s.push(b"first\r");
        let b = s.push(b"\nsecond\n");
        assert_eq!(format!("{a}{b}"), "first\nsecond\n");
    }

    /// 탭과 줄바꿈은 살아남는다. 나머지 C0는 지운다.
    #[test]
    fn tabs_and_newlines_survive_but_other_controls_do_not() {
        let mut s = Stripper::default();
        assert_eq!(s.push(b"a\tb\nc\x07d"), "a\tb\n");
        assert_eq!(s.flush(), "cd");
    }
}
