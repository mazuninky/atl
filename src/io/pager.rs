//! Pager process management.
//!
//! Resolves the pager command from environment variables (`$ATL_PAGER`,
//! `$PAGER`) with a platform-specific fallback (`less -FRX` on Unix, `more`
//! on Windows) and spawns the process with its stdin connected to a pipe.
//!
//! The caller is responsible for feeding the pipe (see
//! [`crate::io::IoStreams::start_pager`]) and for closing it via
//! [`PagerProcess::wait`] so the pager exits cleanly.

use std::io::Write;
use std::process::{Child, Command, Stdio};

use anyhow::Result;

/// Environment variable that forces the pager off when set to `1` / `true`.
const NO_PAGER_ENV: &str = "ATL_NO_PAGER";

/// A running pager process whose stdin is a [`std::process::ChildStdin`] pipe.
///
/// Dropping the struct without calling [`PagerProcess::wait`] will still close
/// the pipe (via `Child`'s drop) but the pager exit is not observed. Prefer
/// explicit [`wait`](Self::wait) calls from `IoStreams::stop_pager`.
pub(crate) struct PagerProcess {
    child: Child,
}

impl PagerProcess {
    /// Takes ownership of the pager's stdin handle, leaving `None` behind on
    /// the child. The caller writes program output through this handle.
    pub(crate) fn take_stdin(&mut self) -> Option<Box<dyn Write + Send>> {
        self.child
            .stdin
            .take()
            .map(|s| Box::new(s) as Box<dyn Write + Send>)
    }

    /// Waits for the pager process to exit. Any non-zero status is swallowed
    /// so the parent program can keep reporting its own result — the pager's
    /// behaviour is not the user's primary concern.
    pub(crate) fn wait(mut self) -> Result<()> {
        let _ = self.child.wait();
        Ok(())
    }
}

/// Spawns the resolved pager command if pager use is allowed.
///
/// Returns `Ok(None)` when the pager is explicitly disabled (via `no_pager`
/// or the `ATL_NO_PAGER` environment variable) or when no pager command can
/// be resolved. Returns `Ok(Some(_))` with a running child whose stdin is a
/// pipe the caller should write into.
pub(crate) fn spawn_pager(no_pager: bool) -> Result<Option<PagerProcess>> {
    if no_pager {
        return Ok(None);
    }
    if env_flag_enabled(NO_PAGER_ENV) {
        return Ok(None);
    }

    let Some(cmd_str) = resolve_pager_command() else {
        return Ok(None);
    };

    let Some(parts) = shlex::split(&cmd_str) else {
        return Ok(None);
    };
    let Some((program, args)) = parts.split_first() else {
        return Ok(None);
    };

    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::piped());

    // `less` needs these defaults to behave like a typical CLI pager when the
    // user has not overridden the environment.
    if program == "less" && std::env::var_os("LESS").is_none() {
        command.env("LESS", "FRX");
    }

    match command.spawn() {
        Ok(child) => Ok(Some(PagerProcess { child })),
        Err(err) => {
            tracing::debug!("pager disabled: failed to spawn `{cmd_str}`: {err}");
            Ok(None)
        }
    }
}

/// Resolves the pager command string, preferring `$ATL_PAGER`, then `$PAGER`,
/// then a platform default.
///
/// This is the live wrapper that reads the environment once and forwards to
/// [`resolve_pager_command_from`], which is pure so unit tests can exercise it
/// with synthetic inputs (no `std::env::set_var` needed).
fn resolve_pager_command() -> Option<String> {
    let atl = std::env::var("ATL_PAGER").ok();
    let pager = std::env::var("PAGER").ok();
    resolve_pager_command_from(atl.as_deref(), pager.as_deref(), cfg!(windows))
}

/// Pure resolver for the pager command string.
///
/// Preference order: `atl_pager` (if non-empty after trimming) →
/// `pager` (if non-empty after trimming) → platform default
/// (`more` on Windows, `less -FRX` elsewhere).
fn resolve_pager_command_from(
    atl_pager: Option<&str>,
    pager: Option<&str>,
    is_windows: bool,
) -> Option<String> {
    if let Some(v) = atl_pager
        && !v.trim().is_empty()
    {
        return Some(v.to_string());
    }
    if let Some(v) = pager
        && !v.trim().is_empty()
    {
        return Some(v.to_string());
    }
    if is_windows {
        Some("more".to_string())
    } else {
        Some("less -FRX".to_string())
    }
}

/// Returns `true` if the given environment variable is set to a truthy value.
fn env_flag_enabled(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => is_truthy(&v),
        Err(_) => false,
    }
}

/// Pure truthiness test for a flag value: non-empty, not `0`, not
/// case-insensitive `false`.
fn is_truthy(value: &str) -> bool {
    let t = value.trim();
    !t.is_empty() && t != "0" && !t.eq_ignore_ascii_case("false")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::env_lock;

    #[test]
    fn env_flag_enabled_variants() {
        let _g = env_lock();
        // SAFETY: env mutation serialized via env_lock().
        unsafe {
            std::env::remove_var("ATL_TEST_FLAG");
            assert!(!env_flag_enabled("ATL_TEST_FLAG"));
            std::env::set_var("ATL_TEST_FLAG", "1");
            assert!(env_flag_enabled("ATL_TEST_FLAG"));
            std::env::set_var("ATL_TEST_FLAG", "true");
            assert!(env_flag_enabled("ATL_TEST_FLAG"));
            std::env::set_var("ATL_TEST_FLAG", "0");
            assert!(!env_flag_enabled("ATL_TEST_FLAG"));
            std::env::set_var("ATL_TEST_FLAG", "false");
            assert!(!env_flag_enabled("ATL_TEST_FLAG"));
            std::env::remove_var("ATL_TEST_FLAG");
        }
    }

    // -------------------------------------------------------------------
    // is_truthy — pure value-classifier; no env access required.
    // -------------------------------------------------------------------

    #[test]
    fn is_truthy_accepts_one() {
        assert!(is_truthy("1"));
    }

    #[test]
    fn is_truthy_accepts_true_lowercase() {
        assert!(is_truthy("true"));
    }

    #[test]
    fn is_truthy_accepts_true_mixed_case() {
        assert!(is_truthy("True"));
        assert!(is_truthy("TRUE"));
    }

    #[test]
    fn is_truthy_accepts_arbitrary_nonzero_strings() {
        // The contract is "anything not 0/false/empty is truthy" — many
        // env-var conventions use `yes`, `on`, etc.
        assert!(is_truthy("yes"));
        assert!(is_truthy("on"));
        assert!(is_truthy("anything"));
    }

    #[test]
    fn is_truthy_rejects_empty() {
        assert!(!is_truthy(""));
    }

    #[test]
    fn is_truthy_rejects_whitespace_only() {
        assert!(!is_truthy("   "));
        assert!(!is_truthy("\t\n"));
    }

    #[test]
    fn is_truthy_rejects_zero() {
        assert!(!is_truthy("0"));
    }

    #[test]
    fn is_truthy_rejects_false_case_insensitive() {
        assert!(!is_truthy("false"));
        assert!(!is_truthy("False"));
        assert!(!is_truthy("FALSE"));
    }

    #[test]
    fn is_truthy_trims_surrounding_whitespace() {
        assert!(is_truthy("  1  "));
        assert!(!is_truthy("  0  "));
        assert!(!is_truthy("  false  "));
    }

    // -------------------------------------------------------------------
    // resolve_pager_command_from — pure resolver, no env access.
    // -------------------------------------------------------------------

    #[test]
    fn resolve_pager_atl_wins_when_present() {
        let r = resolve_pager_command_from(Some("bat -p"), Some("less"), false);
        assert_eq!(r.as_deref(), Some("bat -p"));
    }

    #[test]
    fn resolve_pager_atl_wins_over_pager_on_windows_too() {
        let r = resolve_pager_command_from(Some("bat -p"), Some("more"), true);
        assert_eq!(r.as_deref(), Some("bat -p"));
    }

    #[test]
    fn resolve_pager_falls_through_to_pager_when_atl_empty() {
        let r = resolve_pager_command_from(Some(""), Some("less"), false);
        assert_eq!(r.as_deref(), Some("less"));
    }

    #[test]
    fn resolve_pager_falls_through_to_pager_when_atl_whitespace_only() {
        let r = resolve_pager_command_from(Some("   "), Some("less"), false);
        assert_eq!(r.as_deref(), Some("less"));
    }

    #[test]
    fn resolve_pager_falls_through_to_pager_when_atl_missing() {
        let r = resolve_pager_command_from(None, Some("less -R"), false);
        assert_eq!(r.as_deref(), Some("less -R"));
    }

    #[test]
    fn resolve_pager_default_unix_is_less_frx() {
        let r = resolve_pager_command_from(None, None, false);
        assert_eq!(r.as_deref(), Some("less -FRX"));
    }

    #[test]
    fn resolve_pager_default_windows_is_more() {
        let r = resolve_pager_command_from(None, None, true);
        assert_eq!(r.as_deref(), Some("more"));
    }

    #[test]
    fn resolve_pager_default_unix_when_pager_is_blank() {
        let r = resolve_pager_command_from(None, Some(""), false);
        assert_eq!(r.as_deref(), Some("less -FRX"));
    }

    #[test]
    fn resolve_pager_default_windows_when_pager_is_blank() {
        let r = resolve_pager_command_from(None, Some("  "), true);
        assert_eq!(r.as_deref(), Some("more"));
    }

    // -------------------------------------------------------------------
    // spawn_pager — top-level guard rails. We can verify the no-pager
    // shortcuts without spawning any process.
    // -------------------------------------------------------------------

    #[test]
    fn spawn_pager_returns_none_when_no_pager_flag_set() {
        // no_pager=true must short-circuit before any env or fs access.
        let result = spawn_pager(true).expect("should not error");
        assert!(result.is_none(), "no_pager=true must yield None");
    }
}
