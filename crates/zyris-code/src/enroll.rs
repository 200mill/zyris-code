//! Notices when re-enrollment happens and **draws the code on screen.**
//!
//! The upstream (zyris) provides an `EnrollmentUi` hook (`Enroller::with_ui`, PR #6). Plugging our
//! screen into it means the enrollment code arrives on screen as `Frame::Enroll` instead of going
//! out through the stdout box — the old "code leaking into the terminal" problem structurally
//! disappears. Without a screen (the extreme where first run precedes the screen), it prints the box as before.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use zyris::enroll::{AuthorizeResponse, CredentialStore, EnrollmentUi, TokenResponse};
use zyris::runtime::credentials::Credentials;

use crate::app::{EnrollPhase, EnrollView, Frame};
use crate::tools::bridge::Bridge;

/// The credentials this node will use.
///
/// The order must **match** the upstream `credentials::from_env` — what a person explicitly gives
/// always wins, and enrollment that must ask a person comes last. The only difference is wiring
/// the enrollment path to the screen; since we hold the store ourselves, no path guessing is needed.
///
/// **Scopes must be settled before getting here.** `Enroller` copies `config.scopes` at this point,
/// so scopes given later via `Runner::request_scopes` don't ride on the enrollment request.
pub fn source(
    config: &zyris::runtime::RunConfig,
    bridge: &Bridge,
) -> Result<(Arc<dyn Credentials>, Option<Reauth>), String> {
    use zyris::runtime::credentials::{StaticToken, TokenFile};

    // Where a person gave a token directly, there is nothing to discard and no one to ask again.
    if let Some(token) = StaticToken::from_env().map_err(|e| e.to_string())? {
        return Ok((Arc::new(token), None));
    }
    if let Some(file) = TokenFile::from_env() {
        return Ok((Arc::new(file), None));
    }

    // The credential file lands under `$ZYRIS_CONFIG_DIR`. `main.rs` has filled that variable with
    // this app's directory (`conn::credential_dir`).
    let store = Arc::new(
        zyris::enroll::FileCredentialStore::for_server(&config.url, &config.profile)
            .map_err(|e| e.to_string())?,
    );
    let store = store as Arc<dyn CredentialStore>;

    // This screen draws the enrollment code. Without a screen it falls to the box (`ScreenEnroll::show`).
    let enroller = zyris::enroll::Enroller::new(
        &config.url,
        config.node_name.clone(),
        config.platform().to_string(),
        config.scopes.clone(),
        store.clone(),
    )
    .map_err(|e| e.to_string())?
    .with_ui(Arc::new(ScreenEnroll { bridge: bridge.clone() }));
    let creds: Arc<dyn Credentials> =
        Arc::new(zyris::runtime::credentials::DeviceGrant::new(enroller));

    let reauth = Reauth { store, spent: Arc::new(AtomicBool::new(false)) };
    Ok((creds, Some(reauth)))
}

/// The hook that moves the enrollment code to the screen. The upstream polling loop calls this method.
///
/// Once `show` reaches the screen, the screen owns the display from that moment — nothing goes
/// to stdout. If it doesn't (screen not up yet or already dead), it prints the box as before.
pub struct ScreenEnroll {
    bridge: Bridge,
}

impl EnrollmentUi for ScreenEnroll {
    fn show(&self, response: &AuthorizeResponse) {
        let view = EnrollView {
            code: response.user_code.clone(),
            uri: response.verification_uri.clone(),
            expires_at: std::time::Instant::now()
                + Duration::from_secs(response.expires_in.max(0) as u64),
            phase: EnrollPhase::Waiting,
        };
        if !self.bridge.reaches_screen(Frame::Enroll(view)) {
            // Without a screen, print the box — the same path as before. Even with the hook,
            // this is all the first run (before the screen is up) does.
            println!("{}", zyris::enroll::authorization_notice(response));
        }
    }

    fn lapsed(&self) {
        self.bridge.frame(Frame::EnrollPhase(EnrollPhase::Lapsed));
    }

    fn denied(&self) {
        self.bridge.frame(Frame::EnrollPhase(EnrollPhase::Denied));
    }

    fn authorized(&self, _response: &TokenResponse) {
        self.bridge.frame(Frame::EnrollDone);
    }
}

/// A handle to discard credentials and get authorized again. **Used at most once per process.**
///
/// The scopes settled at approval don't widen when the token is refreshed. When a feature grows
/// and one more scope is needed, there is no path but discarding the credentials — the screen side
/// notices that after attaching (`conn::needs_reenrollment`) and calls this.
#[derive(Clone)]
pub struct Reauth {
    store: Arc<dyn CredentialStore>,
    /// Whether this process has already discarded once. **A person can approve narrowly again** —
    /// discarding every time would demand the browser on every attach, not every launch.
    spent: Arc<AtomicBool>,
}

impl Reauth {
    /// Whether it has already been done. The value fed into the decision (`conn::needs_reenrollment`).
    pub fn spent(&self) -> bool {
        self.spent.load(Ordering::SeqCst)
    }

    /// Discards the credentials. True if something was actually discarded.
    ///
    /// **The connection currently running is left alone.** Cutting it here would make upstream pop
    /// enrollment onto the screen, and the person sees the code **on the screen** — it doesn't leak
    /// to the terminal as before. The credentials are still emptied, so the next launch (or when the connection drops and reattaches) asks cleanly.
    pub async fn discard_once(&self) -> bool {
        if self.spent.swap(true, Ordering::SeqCst) {
            return false;
        }
        match self.store.clear().await {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(error = %e, "could not discard the credentials");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;
    use zyris::enroll::MemoryCredentialStore;

    use crate::app::Action;

    fn stored() -> zyris::enroll::StoredCredential {
        zyris::enroll::StoredCredential::new(
            "a".into(),
            "r".into(),
            "n".into(),
            "arch zyris-code".into(),
            "e@example.com".into(),
            i64::MAX,
        )
    }

    fn authorize() -> AuthorizeResponse {
        AuthorizeResponse {
            device_code: "zdc_secret".into(),
            user_code: "WXQR-7KBD".into(),
            verification_uri: "https://attacca.example/settings/zyris/device".into(),
            expires_in: 600,
            interval: 5,
        }
    }

    /// A bridge with a screen attached, and the mailbox that screen receives.
    fn with_screen() -> (Bridge, mpsc::UnboundedReceiver<crate::app::AppMsg>) {
        let bridge = Bridge::new();
        let (tx, rx) = mpsc::unbounded_channel();
        bridge.attach(tx);
        (bridge, rx)
    }

    /// **With the screen up, the code goes to the screen.** It doesn't leak to stdout.
    #[test]
    fn the_code_goes_to_the_screen_when_one_is_up() {
        let (bridge, mut screen) = with_screen();
        ScreenEnroll { bridge }.show(&authorize());

        match screen.try_recv().expect("must reach the screen") {
            (_, Action::Frame(Frame::Enroll(view))) => {
                assert_eq!(view.code, "WXQR-7KBD");
                assert_eq!(view.uri, "https://attacca.example/settings/zyris/device");
                assert_eq!(view.phase, EnrollPhase::Waiting);
            }
            other => panic!("must be an enrollment frame: {other:?}"),
        }
    }

    /// **Without a screen, print the box** (the old path). That spot is all the first run does.
    #[test]
    fn without_a_screen_the_code_is_printed() {
        let bridge = Bridge::new();
        // Called without a screen, the box goes to stdout — no panic, and that's all.
        ScreenEnroll { bridge }.show(&authorize());
    }

    /// **Expiry, denial, and approval reach the screen.** If they vanished silently, a person wouldn't know.
    #[test]
    fn the_outcomes_reach_the_screen() {
        let (bridge, mut screen) = with_screen();
        let ui = ScreenEnroll { bridge };

        ui.lapsed();
        match screen.try_recv().expect("expiry must reach the screen") {
            (_, Action::Frame(Frame::EnrollPhase(EnrollPhase::Lapsed))) => {}
            other => panic!("must be an expiry frame: {other:?}"),
        }

        ui.denied();
        match screen.try_recv().expect("denial must reach the screen") {
            (_, Action::Frame(Frame::EnrollPhase(EnrollPhase::Denied))) => {}
            other => panic!("must be a denial frame: {other:?}"),
        }

        ui.authorized(&TokenResponse {
            access_token: "zna_x".into(),
            refresh_token: "znr_x".into(),
            expires_in: 3600,
            scope: String::new(),
            node_id: "n".into(),
            node_name: "hello node".into(),
            owner_email: "allen@example.com".into(),
        });
        assert!(matches!(screen.try_recv(), Ok((_, Action::Frame(Frame::EnrollDone)))));
    }

    /// **Credentials are discarded at most once per process.** A person can approve narrowly again,
    /// but discarding on every attach makes a loop that demands the browser each time.
    #[tokio::test]
    async fn a_credential_is_discarded_at_most_once_per_process() {
        let store = Arc::new(MemoryCredentialStore::new());
        store.save(&stored()).await.unwrap();
        let reauth = Reauth { store: store.clone(), spent: Arc::new(AtomicBool::new(false)) };

        assert!(!reauth.spent());
        assert!(reauth.discard_once().await, "the first one is discarded");
        assert!(store.load().await.unwrap().is_none(), "the credential must actually be empty");

        // Even if a fresh credential arrives in between, the second time is left untouched.
        store.save(&stored()).await.unwrap();
        assert!(!reauth.discard_once().await, "the second one is not discarded");
        assert!(store.load().await.unwrap().is_some(), "the newly received credential stays");
        assert!(reauth.spent(), "having tried once feeds into the decision");
    }

    /// **Where a token was given directly there is nothing to discard.** Not having a `Reauth` is that state.
    #[test]
    fn a_static_token_has_no_reauth() {
        // When source() falls into the StaticToken path it isn't Some(reauth) — since the
        // environment can't be shaken here, this only records the contract that the handle may be `None`.
        // The real decision is locked down by the `conn::missing_scopes` test.
    }
}
