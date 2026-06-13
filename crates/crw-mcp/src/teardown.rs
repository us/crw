//! Process teardown: signal handling + the single consolidated exit path.
//!
//! Mirror of `crw-cli`'s `teardown` module (the standalone `crw-mcp` binary
//! cannot depend on the `crw-cli` crate). `main` is the *only* place that
//! exits (via [`finish`]), and `kill_all_browsers()` runs exactly once on
//! every path (Ok, Err, signal, stdin-EOF) before the process dies. This is
//! what structurally closes the "`process::exit` after a browser spawned
//! bypasses `Drop`" leak class.

use std::sync::atomic::{AtomicBool, Ordering};

/// A command-level failure carrying the exit `code` plus an optional message
/// already formatted for stderr. The dispatcher prints `msg` only when present.
#[derive(Debug)]
pub struct CmdError {
    pub code: i32,
    pub msg: Option<String>,
}

impl CmdError {
    /// Exit with `code`; the call site already printed to stderr.
    pub fn code_only(code: i32) -> Self {
        Self { code, msg: None }
    }
}

/// Set once teardown has begun so the signal task, a normal exit, and the
/// stdin-EOF path don't double-run `kill_all_browsers()`.
static TEARING_DOWN: AtomicBool = AtomicBool::new(false);

/// Run `kill_all_browsers()` at most once across all callers. In a proxy-only
/// build (no `embedded` feature) there is no browser engine compiled in, so this
/// is a cheap no-op guard with nothing to kill.
fn teardown_once() {
    if TEARING_DOWN.swap(true, Ordering::SeqCst) {
        return;
    }
    #[cfg(feature = "embedded")]
    crw_renderer::browser::kill_all_browsers();
}

/// Install the signal teardown task. Call once at command entry, **before**
/// any browser spawn or auto-download. On SIGINT/SIGTERM/SIGHUP/SIGQUIT it
/// kills every spawned browser group then exits `128 + signo` (130 SIGINT,
/// 143 SIGTERM, 129 SIGHUP, 131 SIGQUIT). Direct exit after teardown — not a
/// signal re-raise (re-raise under tokio races a second signal).
#[cfg(unix)]
pub fn install_signal_teardown() {
    use tokio::signal::unix::{SignalKind, signal};
    tokio::spawn(async move {
        let mut sigint = signal(SignalKind::interrupt()).expect("install SIGINT handler");
        let mut sigterm = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        let mut sighup = signal(SignalKind::hangup()).expect("install SIGHUP handler");
        let mut sigquit = signal(SignalKind::quit()).expect("install SIGQUIT handler");
        let code = tokio::select! {
            _ = sigint.recv()  => 130,
            _ = sigterm.recv() => 143,
            _ = sighup.recv()  => 129,
            _ = sigquit.recv() => 131,
        };
        teardown_once();
        std::process::exit(code);
    });
}

#[cfg(not(unix))]
pub fn install_signal_teardown() {}

/// The single consolidated exit point. Runs teardown exactly once, prints any
/// error message to stderr, and exits with the right code. Called from `main`.
pub fn finish(result: Result<(), CmdError>) -> ! {
    teardown_once();
    match result {
        Ok(()) => std::process::exit(0),
        Err(CmdError { code, msg }) => {
            if let Some(m) = msg {
                eprintln!("{m}");
            }
            std::process::exit(code);
        }
    }
}
