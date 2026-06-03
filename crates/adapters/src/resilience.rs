//! Shared guardrails for blocking local adapter probes.
//!
//! Adapter discovery is intentionally best-effort: a wedged home directory, SQLite store, or
//! vendor CLI must not stall the popover. These helpers bound one blocking probe and briefly gate
//! retries after a timeout/panic so the refresh loop fails closed instead of piling up work.
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

const LOCAL_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const LOCAL_PROBE_BACKOFF: Duration = Duration::from_secs(30);
const COMMAND_POLL: Duration = Duration::from_millis(10);

#[derive(Clone, Copy)]
pub(crate) enum BlockingProbe {
    OmpProfiles,
    OmpDb,
    CodexAuth,
    CodexConfig,
    CodexVersion,
    ClaudeCredentialsFile,
    ClaudeFilePresence,
    ClaudeVersion,
    #[cfg(target_os = "macos")]
    ClaudeKeychainRead,
    #[cfg(test)]
    TestProbe,
    #[cfg(target_os = "macos")]
    ClaudeKeychainPresence,
}

struct ProbeGate {
    blocked_until: Option<Instant>,
}

impl ProbeGate {
    const fn new() -> Self {
        Self {
            blocked_until: None,
        }
    }
}

static OMP_PROFILES_GATE: Mutex<ProbeGate> = parking_lot::const_mutex(ProbeGate::new());
static OMP_DB_GATE: Mutex<ProbeGate> = parking_lot::const_mutex(ProbeGate::new());
static CODEX_AUTH_GATE: Mutex<ProbeGate> = parking_lot::const_mutex(ProbeGate::new());
static CODEX_CONFIG_GATE: Mutex<ProbeGate> = parking_lot::const_mutex(ProbeGate::new());
static CODEX_VERSION_GATE: Mutex<ProbeGate> = parking_lot::const_mutex(ProbeGate::new());
static CLAUDE_CREDENTIALS_FILE_GATE: Mutex<ProbeGate> = parking_lot::const_mutex(ProbeGate::new());
static CLAUDE_FILE_PRESENCE_GATE: Mutex<ProbeGate> = parking_lot::const_mutex(ProbeGate::new());
static CLAUDE_VERSION_GATE: Mutex<ProbeGate> = parking_lot::const_mutex(ProbeGate::new());
#[cfg(target_os = "macos")]
static CLAUDE_KEYCHAIN_READ_GATE: Mutex<ProbeGate> = parking_lot::const_mutex(ProbeGate::new());
#[cfg(test)]
static TEST_PROBE_GATE: Mutex<ProbeGate> = parking_lot::const_mutex(ProbeGate::new());
#[cfg(target_os = "macos")]
static CLAUDE_KEYCHAIN_PRESENCE_GATE: Mutex<ProbeGate> = parking_lot::const_mutex(ProbeGate::new());

enum ProbeOutcome<T> {
    Done(Option<T>),
    Failed,
}

enum CommandOutcome {
    Done { success: bool, stdout: Vec<u8> },
    Failed,
}

impl BlockingProbe {
    fn gate(self) -> &'static Mutex<ProbeGate> {
        match self {
            Self::OmpProfiles => &OMP_PROFILES_GATE,
            Self::OmpDb => &OMP_DB_GATE,
            Self::CodexAuth => &CODEX_AUTH_GATE,
            Self::CodexConfig => &CODEX_CONFIG_GATE,
            Self::CodexVersion => &CODEX_VERSION_GATE,
            Self::ClaudeCredentialsFile => &CLAUDE_CREDENTIALS_FILE_GATE,
            Self::ClaudeFilePresence => &CLAUDE_FILE_PRESENCE_GATE,
            Self::ClaudeVersion => &CLAUDE_VERSION_GATE,
            #[cfg(target_os = "macos")]
            Self::ClaudeKeychainRead => &CLAUDE_KEYCHAIN_READ_GATE,
            #[cfg(test)]
            Self::TestProbe => &TEST_PROBE_GATE,
            #[cfg(target_os = "macos")]
            Self::ClaudeKeychainPresence => &CLAUDE_KEYCHAIN_PRESENCE_GATE,
        }
    }
}

pub(crate) fn bounded_blocking_probe<T, F>(probe: BlockingProbe, f: F) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce() -> Option<T> + Send + 'static,
{
    bounded_blocking_probe_with_timeout(probe, LOCAL_PROBE_TIMEOUT, f)
}

pub(crate) fn command_stdout(
    probe: BlockingProbe,
    program: &str,
    args: &[&str],
) -> Option<Vec<u8>> {
    if probe_is_blocked(probe) {
        return None;
    }

    match command_stdout_with_timeout(program, args, LOCAL_PROBE_TIMEOUT) {
        CommandOutcome::Done { success, stdout } => {
            record_probe_success(probe);
            success.then_some(stdout)
        }
        CommandOutcome::Failed => {
            record_probe_failure(probe);
            None
        }
    }
}

fn bounded_blocking_probe_with_timeout<T, F>(
    probe: BlockingProbe,
    timeout: Duration,
    f: F,
) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce() -> Option<T> + Send + 'static,
{
    if probe_is_blocked(probe) {
        return None;
    }

    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let outcome = match catch_unwind(AssertUnwindSafe(f)) {
            Ok(value) => ProbeOutcome::Done(value),
            Err(_) => ProbeOutcome::Failed,
        };
        let _ = tx.send(outcome);
    });

    match rx.recv_timeout(timeout) {
        Ok(ProbeOutcome::Done(value)) => {
            record_probe_success(probe);
            value
        }
        Ok(ProbeOutcome::Failed) | Err(_) => {
            record_probe_failure(probe);
            None
        }
    }
}

fn command_stdout_with_timeout(program: &str, args: &[&str], timeout: Duration) -> CommandOutcome {
    let Ok(mut child) = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return CommandOutcome::Failed;
    };

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let Ok(output) = child.wait_with_output() else {
                    return CommandOutcome::Failed;
                };
                return CommandOutcome::Done {
                    success: status.success(),
                    stdout: output.stdout,
                };
            }
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return CommandOutcome::Failed;
            }
        }

        let elapsed = started.elapsed();
        if elapsed >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return CommandOutcome::Failed;
        }
        thread::sleep(COMMAND_POLL.min(timeout - elapsed));
    }
}

fn probe_is_blocked(probe: BlockingProbe) -> bool {
    let now = Instant::now();
    let mut gate = probe.gate().lock();
    if let Some(until) = gate.blocked_until {
        if now < until {
            return true;
        }
        gate.blocked_until = None;
    }
    false
}

fn record_probe_success(probe: BlockingProbe) {
    probe.gate().lock().blocked_until = None;
}

fn record_probe_failure(probe: BlockingProbe) {
    probe.gate().lock().blocked_until = Some(Instant::now() + LOCAL_PROBE_BACKOFF);
}

#[cfg(test)]
fn reset_probe_gate(probe: BlockingProbe) {
    probe.gate().lock().blocked_until = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_probe_returns_completed_value() {
        reset_probe_gate(BlockingProbe::TestProbe);

        let value = bounded_blocking_probe_with_timeout(
            BlockingProbe::TestProbe,
            Duration::from_secs(1),
            || Some(7),
        );

        assert_eq!(value, Some(7));
    }

    #[test]
    fn bounded_probe_timeout_gates_follow_up() {
        reset_probe_gate(BlockingProbe::TestProbe);

        let timed_out = bounded_blocking_probe_with_timeout(
            BlockingProbe::TestProbe,
            Duration::from_millis(10),
            || {
                thread::sleep(Duration::from_millis(100));
                Some(7)
            },
        );
        let gated = bounded_blocking_probe_with_timeout(
            BlockingProbe::TestProbe,
            Duration::from_secs(1),
            || Some(9),
        );

        reset_probe_gate(BlockingProbe::TestProbe);
        assert_eq!(timed_out, None);
        assert_eq!(gated, None);
    }
}
