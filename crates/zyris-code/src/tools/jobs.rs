//! 배경에 건 명령. **띄우고, 모으고, 그룹째 죽인다.**
//!
//! 여기는 프로세스와 버퍼만 안다. 와이어 표면(마감 계약·`until`의 갈래)은 `wait.rs`다.
//! 둘을 가른 이유는 테스트다 — 한 파일이면 마감을 재는 테스트마저 진짜 프로세스를
//! 띄워야 한다.

use std::sync::LazyLock;

use regex::Regex;

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
