//! Produces the screen's words in Korean and English. Changed with `/lang`.
//!
//! **All phrases are gathered in this one place.** Split per-widget with conditionals, one language
//! would inevitably get edited alone, leaving half the screen in the other language. With a single
//! function here holding both languages side by side, both are in view when editing.
//!
//! ## Why it lives in two places
//!
//! - `State.lang` — used by the drawing side. Since `apply` must stay pure, it has to be carried as
//!   state, and screen tests being able to fix a language and look at it is thanks to this too.
//! - `lang::current()` — used where there is no screen (the shell notice in `notice.rs`, errors the
//!   tools return). Carrying it as an argument that far would string a `lang` through functions that aren't even pure.
//!
//! `/lang` sets both together. If they diverged, the conversation window would be English while only the shell notice stayed Korean.
//!
//! ## Which words come here
//!
//! **Only what people read.** Tool descriptions read by the agent are always English (the doc
//! comments in `tools/`), and code comments and test names are always Korean. What this file divides is only the screen.

use std::sync::atomic::{AtomicU8, Ordering};

/// **The default is English.** This repo is written in Korean, but the people receiving the app
/// aren't — a screen in a language they can't read makes it unusable. Korean comes from the locale or a person's choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    Ko,
    #[default]
    En,
}

/// The current language. Used where there is no screen.
static CURRENT: AtomicU8 = AtomicU8::new(1);

pub fn current() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        0 => Lang::Ko,
        _ => Lang::En,
    }
}

pub fn set(lang: Lang) {
    CURRENT.store(
        match lang {
            Lang::Ko => 0,
            Lang::En => 1,
        },
        Ordering::Relaxed,
    );
}

impl Lang {
    /// From what the person typed. **Both languages' names are accepted** — typing `/lang` with a
    /// Korean word on an English screen is natural, and so is `/lang english` on a Korean one.
    pub fn parse(text: &str) -> Option<Lang> {
        match text.trim().to_ascii_lowercase().as_str() {
            "ko" | "kr" | "korean" | "한글" | "한국어" => Some(Lang::Ko),
            "en" | "eng" | "english" | "영어" => Some(Lang::En),
            _ => None,
        }
    }

    /// The name written to the setting.
    pub fn code(self) -> &'static str {
        match self {
            Lang::Ko => "ko",
            Lang::En => "en",
        }
    }

    /// The name shown to people. **Written in its own language** — since it's for picking from a
    /// list, a name in a language you can't read right now leaves you unable to tell what you'd choose.
    pub fn name(self) -> &'static str {
        match self {
            Lang::Ko => "한국어",
            Lang::En => "English",
        }
    }

    fn pick(self, ko: &'static str, en: &'static str) -> &'static str {
        match self {
            Lang::Ko => ko,
            Lang::En => en,
        }
    }
}

/// Which language to start with at launch.
///
/// Order: `$ZYRIS_CODE_LANG` → last choice → system locale → Korean.
///
/// **What a person gave always wins.** The last choice comes next because a `/lang` change must
/// survive into the next run to count as a "setting".
pub fn startup() -> Lang {
    if let Some(given) = std::env::var("ZYRIS_CODE_LANG").ok().and_then(|v| Lang::parse(&v)) {
        return given;
    }
    if let Some(saved) = load() {
        return saved;
    }
    from_locale(std::env::var("LC_ALL").or_else(|_| std::env::var("LANG")).ok().as_deref())
}

/// `ko_KR.UTF-8` → Korean. An unknown locale is treated as English — offering a Korean screen to
/// someone who can't read Korean is worse than the reverse.
pub fn from_locale(locale: Option<&str>) -> Lang {
    match locale {
        Some(l) if l.to_ascii_lowercase().starts_with("ko") => Lang::Ko,
        Some(l) if !l.trim().is_empty() => Lang::En,
        // Environments with no locale at all (docker, systemd) stay at the default, English.
        _ => Lang::En,
    }
}

/// The file where the chosen language lives. Same directory as the credentials.
fn store() -> Option<std::path::PathBuf> {
    crate::conn::credential_dir().map(|dir| dir.join("lang"))
}

pub fn load() -> Option<Lang> {
    Lang::parse(&std::fs::read_to_string(store()?).ok()?)
}

/// Saves the choice. **The app keeps running even if this fails** — it's already changed for this run.
pub fn save(lang: Lang) {
    let Some(at) = store() else { return };
    if let Some(dir) = at.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&at, lang.code()) {
        tracing::warn!(error = %e, "고른 언어를 남기지 못했다");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Screen phrases
// ─────────────────────────────────────────────────────────────────────────────

impl Lang {
    // ── Bottom bar · activity line
    pub fn mode_normal(self) -> &'static str {
        self.pick("기본", "normal")
    }
    pub fn mode_plan(self) -> &'static str {
        self.pick("계획", "plan")
    }
    /// **Not translated.** These two must stay exactly the names attacca uses on its own screen, so
    /// what's opened here can be found in that list — same reason sessions are written as `thread`.
    /// Moved to the Korean for "work", it would blend with the "working…" just above in the activity line and blur what each points at.
    pub fn mode_work(self) -> &'static str {
        "work"
    }
    pub fn mode_job(self) -> &'static str {
        "job"
    }
    pub fn working(self) -> &'static str {
        self.pick("작업 중…", "Working…")
    }
    pub fn stopping(self) -> &'static str {
        self.pick("멈추는 중…", "Stopping…")
    }
    pub fn idle(self) -> &'static str {
        self.pick("쉬는 중", "Taking a break")
    }
    pub fn esc_stops(self) -> &'static str {
        self.pick("Esc 정지", "Esc stops")
    }
    pub fn ctrl_c_quits(self) -> &'static str {
        self.pick("Ctrl+C 종료", "Ctrl+C quits")
    }
    pub fn queued(self, n: usize) -> String {
        match self {
            Lang::Ko => format!("대기 {n}개"),
            Lang::En => format!("{n} queued"),
        }
    }
    pub fn quit_armed(self) -> &'static str {
        self.pick("한 번 더 Ctrl+C를 누르면 끝냅니다", "Press Ctrl+C again to quit")
    }
    /// Connected. The activity line shows this briefly, then settles to `idle()` —
    /// the transition a user sees is connecting → connected → taking a break.
    pub fn connected(self) -> &'static str {
        self.pick("연결됨", "Connected")
    }
    /// What to show in the activity line while a command runs.
    pub fn running_command(self, command: &str, secs: u64) -> String {
        match self {
            Lang::Ko => format!("▶ {command}  ·  {secs}초"),
            Lang::En => format!("▶ {command}  ·  {secs}s"),
        }
    }
    /// What to show in the activity line while waiting for a question.
    pub fn waiting_answer(self) -> &'static str {
        self.pick("대기 중 — 답을 고르세요", "Waiting — answer the question")
    }
    /// The hint attached to the right of the activity line while waiting for a question.
    pub fn waiting_answer_hint(self) -> &'static str {
        self.pick("↑↓ 이동 · Enter 고르기", "↑↓ move · Enter choose")
    }

    pub fn mode_now(self, mode: &str) -> String {
        match self {
            Lang::Ko => format!(
                "지금은 **{mode}** 모드입니다. Shift+Tab으로 돌리거나 \
                 `/mode 기본`·`/mode 계획`·`/mode work`·`/mode job`으로 바꿉니다."
            ),
            Lang::En => format!(
                "Mode is **{mode}**. Cycle it with Shift+Tab, or set it with \
                 `/mode normal`, `/mode plan`, `/mode work`, `/mode job`."
            ),
        }
    }
    pub fn mode_changed(self, mode: &str) -> String {
        match self {
            Lang::Ko => format!("**{mode}** 모드로 바꿨습니다."),
            Lang::En => format!("Mode is now **{mode}**."),
        }
    }

    /// When entering `work`·`job`. **It says in advance what the next message becomes** — if you
    /// think only the mode changed and keep writing your ongoing talk, that becomes the goal.
    pub fn mode_opens_work(self) -> &'static str {
        self.pick(
            "다음에 보내는 말이 **work의 목표**가 됩니다. attacca가 계획을 세워 \
             태스크로 쪼갭니다 — 관문 둘은 사람이 열어야 합니다.",
            "Your next message becomes a **work goal**. Attacca plans it into tasks; \
             the two gates need a person to open them.",
        )
    }
    pub fn mode_opens_job(self) -> &'static str {
        self.pick(
            "다음에 보내는 말이 **job**이 됩니다. 시켜 놓으면 끝까지 해냅니다 — \
             되묻는 것이 있으면 그대로 답하면 됩니다.",
            "Your next message becomes a **job** — hand it over and it runs to the end. \
             If it asks something back, just answer here.",
        )
    }

    /// When switching to work mode with a session open. **The mode no longer opens new things** —
    /// the ongoing conversation continues as-is, and a new work opens in a new thread.
    pub fn mode_continues_work(self) -> &'static str {
        self.pick(
            "모드가 **work**입니다. 지금 대화는 그대로 이어갑니다 — \
             새 work는 ←의 새 쓰레드에서 엽니다.",
            "Mode is **work**. This conversation continues as-is — \
             start a new work from a new thread (←).",
        )
    }
    pub fn mode_continues_job(self) -> &'static str {
        self.pick(
            "모드가 **job**입니다. 지금 대화는 그대로 이어갑니다 — \
             새 job은 ←의 새 쓰레드에서 엽니다.",
            "Mode is **job**. This conversation continues as-is — \
             start a new job from a new thread (←).",
        )
    }

    /// After opening. **It says which one opened by id** — that's what's needed to find it on the attacca side.
    pub fn opened_work(self, id: &str) -> String {
        match self {
            Lang::Ko => format!("work **{id}**을 열었습니다. 여기서 계획을 두고 얘기하면 됩니다."),
            Lang::En => format!("Opened work **{id}**. Talk the plan over right here."),
        }
    }
    pub fn opened_job(self, id: &str) -> String {
        match self {
            Lang::Ko => format!("job **{id}**을 걸었습니다. 도는 것을 여기서 봅니다."),
            Lang::En => format!("Queued job **{id}**. Watch it run right here."),
        }
    }

    pub fn connecting(self) -> &'static str {
        self.pick("연결 중…", "Connecting…")
    }
    pub fn disconnected(self, why: &str) -> String {
        match self {
            Lang::Ko => format!("연결이 끊겼습니다 ({why}). 다시 붙는 중입니다."),
            Lang::En => format!("Disconnected ({why}). Reconnecting."),
        }
    }

    // ── Lists (picker)
    pub fn new_thread(self) -> &'static str {
        self.pick("＋ 새 쓰레드", "+ New thread")
    }
    pub fn projects(self) -> &'static str {
        self.pick("프로젝트", "Projects")
    }
    pub fn threads_in(self, project: &str) -> String {
        match self {
            Lang::Ko => format!("쓰레드  ·  {project}"),
            Lang::En => format!("Threads  ·  {project}"),
        }
    }
    pub fn agents(self) -> &'static str {
        self.pick("에이전트", "Agents")
    }
    pub fn commands(self) -> &'static str {
        self.pick("명령", "Commands")
    }
    pub fn language(self) -> &'static str {
        self.pick("화면 말", "Language")
    }
    /// The mark attached to the language currently in use, in the list.
    pub fn in_use(self) -> &'static str {
        self.pick("지금", "in use")
    }
    pub fn new_project(self) -> &'static str {
        self.pick("＋ 새 프로젝트", "+ New project")
    }
    /// Choosing it opens a form for the name and description — the list has no place to type.
    pub fn new_project_note(self) -> &'static str {
        self.pick("이름과 설명을 적습니다", "type a name and description")
    }
    // ── New project form
    pub fn project_form_title(self) -> &'static str {
        self.pick("새 프로젝트", "New project")
    }
    pub fn project_name(self) -> &'static str {
        self.pick("이름", "Name")
    }
    pub fn project_name_placeholder(self) -> &'static str {
        self.pick("프로젝트 이름", "project name")
    }
    pub fn project_description(self) -> &'static str {
        self.pick("설명", "Description")
    }
    pub fn project_description_placeholder(self) -> &'static str {
        self.pick("무엇을 하는 곳인지", "what it is for")
    }
    pub fn project_form_keys(self) -> &'static str {
        self.pick(
            "Tab 다음 칸 · Enter 만들기 · Esc 닫기",
            "Tab next field · Enter create · Esc close",
        )
    }
    /// **An empty name isn't created** — it's unclear what's being made, and a nameless row in the
    /// list would have no way to be removed.
    pub fn project_name_required(self) -> &'static str {
        self.pick("이름을 적어 주세요.", "Type a name.")
    }
    pub fn project_created(self, name: &str) -> String {
        match self {
            Lang::Ko => format!(
                "프로젝트 **{name}**을 만들고 그 안으로 들어왔습니다. \
                 여기서 여는 thread·job·work는 이 프로젝트의 것이 됩니다."
            ),
            Lang::En => format!(
                "Created project **{name}** and moved into it. \
                 Threads, jobs and works you open here belong to it."
            ),
        }
    }
    pub fn default_project(self) -> &'static str {
        self.pick("기본", "default")
    }
    pub fn running(self) -> &'static str {
        self.pick("작업 중", "running")
    }

    pub fn unknown_command(self, what: &str, help: &str) -> String {
        match self {
            Lang::Ko => format!("`/{what}`은 모르는 명령입니다.\n\n{help}"),
            Lang::En => format!("`/{what}` is not a command.\n\n{help}"),
        }
    }

    // ── Sidebar
    pub fn usage(self) -> &'static str {
        self.pick("사용량", "Usage")
    }
    pub fn credits(self) -> &'static str {
        self.pick("크레딧", "Credits")
    }
    pub fn context(self) -> &'static str {
        self.pick("컨텍스트", "Context")
    }
    pub fn total_tokens(self) -> &'static str {
        self.pick("총 토큰", "Tokens")
    }
    pub fn shells(self) -> &'static str {
        self.pick("셸", "Shells")
    }
    pub fn tasks(self) -> &'static str {
        self.pick("태스크", "Tasks")
    }
    pub fn none(self) -> &'static str {
        self.pick("없음", "None")
    }

    // ── Question screen
    pub fn type_your_own(self) -> &'static str {
        self.pick("✎ 직접 입력", "✎ Type your own")
    }
    pub fn type_here(self) -> &'static str {
        self.pick("여기에 직접 적으세요 (Enter로 확정)", "Type here (Enter to confirm)")
    }
    pub fn typing_keys(self) -> &'static str {
        self.pick("Enter 입력 끝 · Esc 취소", "Enter to finish · Esc to cancel")
    }
    pub fn choosing_keys(self) -> &'static str {
        self.pick(
            "↑↓ 이동 · Enter 고르기/실행 · 클릭도 됨 · Esc 접기",
            "↑↓ move · Enter choose/run · click works too · Esc folds",
        )
    }
    pub fn review_keys(self) -> &'static str {
        self.pick("↑↓ 이동 · Enter 실행 · 클릭도 됨", "↑↓ move · Enter runs · click works too")
    }
    pub fn answered(self) -> &'static str {
        self.pick("답한 내용", "Your answer")
    }
    pub fn skipped(self) -> &'static str {
        self.pick("건너뜀", "skipped")
    }

    // ── Approval screen
    pub fn approve_keys(self) -> &'static str {
        self.pick(
            "  y 허용 / n 거부 / a 이 디렉터리는 이번 쓰레드 내내 허용",
            "  y allow / n deny / a allow this directory for the whole thread",
        )
    }
    pub fn approve_head(self) -> &'static str {
        self.pick(
            "작업 디렉터리 밖입니다. 승인이 필요합니다",
            "Outside the working directory. This needs your approval",
        )
    }
    pub fn approve_root(self, cwd: &str) -> String {
        match self {
            Lang::Ko => format!("여기서 도는 곳은 {cwd}"),
            Lang::En => format!("Tools run in {cwd}"),
        }
    }
    pub fn approve_more_waiting(self, n: usize) -> String {
        match self {
            Lang::Ko => format!("  뒤에 {n}개가 더 기다립니다"),
            Lang::En => format!("  {n} more waiting behind this"),
        }
    }
    pub fn approve_gave_up(self) -> &'static str {
        self.pick("  기다리다 돌아갔습니다", "  The server gave up waiting")
    }
    pub fn approve_next_time(self) -> &'static str {
        self.pick(
            "  허용하면 다음 시도에 바로 실행됩니다",
            "  Allowing it runs on the next attempt",
        )
    }
    pub fn approve_expired(self) -> &'static str {
        self.pick(
            "서버가 이 호출을 포기했습니다. 허용하면 다음 호출부터 적용됩니다.",
            "The server gave up on this call. Allowing it applies from the next one.",
        )
    }

    // ── Enrollment code window
    pub fn enroll_title(self) -> &'static str {
        self.pick("Attacca 연결", "Connect to Attacca")
    }
    pub fn enroll_steps(self) -> &'static str {
        self.pick(
            "브라우저에서 이 주소를 열고, 아래 코드를 입력해 승인해 주세요:",
            "Open this address in your browser and enter the code below:",
        )
    }
    pub fn enroll_expires(self, secs: u64) -> String {
        let minutes = secs.div_ceil(60);
        match self {
            Lang::Ko => format!("코드는 {minutes}분 후 만료됩니다."),
            Lang::En => format!("Code expires in {minutes} minute(s)."),
        }
    }
    pub fn enroll_lapsed(self) -> &'static str {
        self.pick(
            "코드가 만료됐습니다. 새 코드를 요청하는 중입니다…",
            "That code expired. Requesting a new one…",
        )
    }
    pub fn enroll_denied(self) -> &'static str {
        self.pick(
            "브라우저에서 거부했습니다. Esc를 눌러 닫으세요.",
            "The request was declined in the browser. Press Esc to close.",
        )
    }
    pub fn enroll_keys(self) -> &'static str {
        self.pick("Esc 닫기", "Esc close")
    }

    // ── `/lang`
    pub fn lang_now(self) -> String {
        match self {
            Lang::Ko => {
                format!("화면 말: **{}**. `/lang en`으로 영어로 바꿉니다.", Lang::Ko.name())
            }
            Lang::En => {
                format!("Interface language: **{}**. Use `/lang ko` for Korean.", Lang::En.name())
            }
        }
    }
    pub fn lang_changed(self) -> &'static str {
        self.pick(
            "화면 말을 한국어로 바꿨습니다. 다음에 켤 때도 이대로입니다.",
            "Interface language is now English. It stays this way next time.",
        )
    }
    pub fn lang_unknown(self, given: &str) -> String {
        match self {
            Lang::Ko => format!("`{given}`가 무슨 말인지 모르겠습니다. `ko` 또는 `en`입니다."),
            Lang::En => format!("Cannot tell what `{given}` means. Use `ko` or `en`."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Both languages' names are accepted.** Typing `/lang` with a Korean word on an English screen
    /// is natural, and so is the reverse — accepting only the current screen language would leave someone who chose wrong with no way back.
    #[test]
    fn either_language_can_be_named_in_either_language() {
        for said in ["ko", "KO", "한글", "한국어", "korean"] {
            assert_eq!(Lang::parse(said), Some(Lang::Ko), "{said}");
        }
        for said in ["en", "English", "영어", " eng "] {
            assert_eq!(Lang::parse(said), Some(Lang::En), "{said}");
        }
        assert_eq!(Lang::parse("일본어"), None);
        assert_eq!(Lang::parse(""), None);
    }

    /// The locale is only a guess. **Unknown means English** — offering a Korean screen to someone
    /// who can't read Korean is worse than the reverse.
    #[test]
    fn the_locale_is_a_guess_that_errs_towards_english() {
        assert_eq!(from_locale(Some("ko_KR.UTF-8")), Lang::Ko);
        assert_eq!(from_locale(Some("KO")), Lang::Ko);
        assert_eq!(from_locale(Some("en_US.UTF-8")), Lang::En);
        assert_eq!(from_locale(Some("fr_FR")), Lang::En);
        assert_eq!(from_locale(None), Lang::En, "모르면 영어다");
        assert_eq!(from_locale(Some("  ")), Lang::En);
    }

    /// The name written and the name read back must match — if they diverge, the saved setting can't be read.
    #[test]
    fn what_is_written_is_what_is_read_back() {
        for lang in [Lang::Ko, Lang::En] {
            assert_eq!(Lang::parse(lang.code()), Some(lang));
        }
    }

    /// **A language names itself in its own language.** Written in a language you can't read now,
    /// you can't tell what you're choosing.
    #[test]
    fn a_language_names_itself() {
        assert_eq!(Lang::Ko.name(), "한국어");
        assert_eq!(Lang::En.name(), "English");
    }

    /// Both languages **must both be present.** Filling only one side leaves half the screen in the
    /// other language.
    ///
    /// **`mode_work`·`mode_job` are deliberately left out.** They're exactly the names attacca uses on
    /// its own screen, so both languages are the same, and putting them here would trip a "not translated" check. `the_english_side_has_no_hangul_left_in_it` below guards them instead.
    #[test]
    fn no_message_is_left_in_one_language_only() {
        let ko = Lang::Ko;
        let en = Lang::En;
        let pairs: Vec<(&str, &str)> = vec![
            (ko.working(), en.working()),
            (ko.idle(), en.idle()),
            (ko.stopping(), en.stopping()),
            (ko.connected(), en.connected()),
            (ko.waiting_answer(), en.waiting_answer()),
            (ko.new_thread(), en.new_thread()),
            (ko.projects(), en.projects()),
            (ko.new_project(), en.new_project()),
            (ko.project_form_title(), en.project_form_title()),
            (ko.project_form_keys(), en.project_form_keys()),
            (ko.project_name_required(), en.project_name_required()),
            (ko.agents(), en.agents()),
            (ko.commands(), en.commands()),
            (ko.mode_normal(), en.mode_normal()),
            (ko.mode_plan(), en.mode_plan()),
            (ko.approve_keys(), en.approve_keys()),
            (ko.esc_stops(), en.esc_stops()),
            (ko.enroll_title(), en.enroll_title()),
            (ko.enroll_steps(), en.enroll_steps()),
            (ko.enroll_lapsed(), en.enroll_lapsed()),
            (ko.enroll_denied(), en.enroll_denied()),
            (ko.enroll_keys(), en.enroll_keys()),
        ];
        for (k, e) in pairs {
            assert_ne!(k, e, "번역이 안 된 문구가 있다: {k}");
            assert!(!k.is_empty() && !e.is_empty());
        }
    }

    /// The English screen must have **not a single Hangul character.** Mixed in, an untranslated spot wouldn't show.
    #[test]
    fn the_english_side_has_no_hangul_left_in_it() {
        let en = Lang::En;
        let said = [
            en.working(),
            en.idle(),
            en.stopping(),
            en.new_thread(),
            en.projects(),
            en.agents(),
            en.commands(),
            en.mode_work(),
            en.mode_job(),
            en.approve_keys(),
            en.approve_expired(),
            en.esc_stops(),
            en.quit_armed(),
            en.lang_changed(),
            en.enroll_title(),
            en.enroll_steps(),
            en.enroll_lapsed(),
            en.enroll_denied(),
            en.enroll_keys(),
            en.connected(),
            en.waiting_answer(),
            en.mode_continues_work(),
            en.mode_continues_job(),
            en.project_form_title(),
            en.project_name(),
            en.project_name_placeholder(),
            en.project_description(),
            en.project_description_placeholder(),
            en.project_form_keys(),
            en.project_name_required(),
        ];
        for text in said {
            assert!(
                !text.chars().any(|c| ('가'..='힣').contains(&c)),
                "영어 문구에 한글이 남았다: {text}"
            );
        }
        assert!(!en.queued(3).chars().any(|c| ('가'..='힣').contains(&c)));
        assert!(!en.threads_in("proj").chars().any(|c| ('가'..='힣').contains(&c)));
    }

    /// In Korean, thread is **sseuredeu**. Keeping the English word would make it stick out in the list.
    #[test]
    fn thread_reads_as_sseurede_in_korean() {
        assert!(Lang::Ko.new_thread().contains("쓰레드"), "{}", Lang::Ko.new_thread());
        assert!(Lang::Ko.threads_in("proj").contains("쓰레드"));
        assert!(Lang::Ko.approve_keys().contains("쓰레드"));
    }
}
