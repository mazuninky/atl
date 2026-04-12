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
fn resolve_pager_command() -> Option<String> {
    if let Ok(v) = std::env::var("ATL_PAGER")
        && !v.trim().is_empty()
    {
        return Some(v);
    }
    if let Ok(v) = std::env::var("PAGER")
        && !v.trim().is_empty()
    {
        return Some(v);
    }
    if cfg!(windows) {
        Some("more".to_string())
    } else {
        Some("less -FRX".to_string())
    }
}

/// Returns `true` if the given environment variable is set to a truthy value.
fn env_flag_enabled(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => {
            let t = v.trim();
            !t.is_empty() && t != "0" && !t.eq_ignore_ascii_case("false")
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_flag_enabled_variants() {
        // SAFETY: env mutation in a single-threaded unit test.
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
}
