//! Abstraction over stdin/stdout/stderr, TTY detection, color handling, and
//! pager integration.
//!
//! All command handlers receive `&mut IoStreams` so tests can substitute
//! buffer-backed streams and so the pager can transparently intercept
//! long-running console output.

mod pager;

use std::io::{self, BufRead, BufReader, Cursor, IsTerminal, Write};
use std::sync::{Arc, Mutex};

use anyhow::Result;

use self::pager::{PagerProcess, spawn_pager};

/// How color output should be selected.
///
/// Resolved once at construction time and cached on [`IoStreams`]; callers
/// query it via [`IoStreams::color_enabled`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorChoice {
    /// Emit color only if stdout is a terminal and no env var forbids it.
    Auto,
    /// Always emit color regardless of TTY state.
    Always,
    /// Never emit color.
    Never,
}

impl ColorChoice {
    /// Resolves the effective color choice from CLI flags, environment
    /// variables, and TTY state.
    ///
    /// Rules, highest priority first:
    /// 1. `no_color_flag` is true → [`ColorChoice::Never`]
    /// 2. `NO_COLOR` env var set (any value) → [`ColorChoice::Never`]
    /// 3. `CLICOLOR_FORCE=1` → [`ColorChoice::Always`]
    /// 4. stdout is a TTY → [`ColorChoice::Auto`]
    /// 5. otherwise → [`ColorChoice::Never`]
    #[must_use]
    pub fn resolve(no_color_flag: bool, is_stdout_tty: bool) -> Self {
        if no_color_flag {
            return ColorChoice::Never;
        }
        if std::env::var_os("NO_COLOR").is_some() {
            return ColorChoice::Never;
        }
        if let Ok(v) = std::env::var("CLICOLOR_FORCE")
            && v == "1"
        {
            return ColorChoice::Always;
        }
        if is_stdout_tty {
            ColorChoice::Auto
        } else {
            ColorChoice::Never
        }
    }

    /// Returns whether color should actually be emitted.
    #[must_use]
    pub fn enabled(self) -> bool {
        matches!(self, ColorChoice::Auto | ColorChoice::Always)
    }
}

/// The current stdout backend: either the real stdout, a pager pipe, or a
/// buffer captured for tests.
enum StdoutBackend {
    System,
    Pager(Box<dyn Write + Send>),
    Buffer(Arc<Mutex<Vec<u8>>>),
}

/// The current stderr backend: either the real stderr or a captured buffer.
enum StderrBackend {
    System,
    Buffer(Arc<Mutex<Vec<u8>>>),
}

/// The current stdin backend: either a line-buffered reader over real stdin,
/// or an in-memory cursor for tests.
enum StdinBackend {
    System(BufReader<io::Stdin>),
    Buffer(Cursor<Vec<u8>>),
}

/// Unified view over stdin/stdout/stderr with color and pager support.
///
/// Command handlers receive `&mut IoStreams`. In tests, construct a
/// buffer-backed instance via [`IoStreams::test`] and inspect the captured
/// output with [`IoStreams::stdout_as_string`] / [`IoStreams::stderr_as_string`].
pub struct IoStreams {
    stdin: StdinBackend,
    stdout: StdoutBackend,
    stderr: StderrBackend,
    is_stdin_tty: bool,
    is_stdout_tty: bool,
    is_stderr_tty: bool,
    color_enabled: bool,
    no_pager: bool,
    pager: Option<PagerProcess>,
}

impl IoStreams {
    /// Constructs an `IoStreams` backed by the real system streams, using
    /// the CLI flags for color and pager configuration.
    pub fn system(cli: &crate::cli::args::Cli) -> Result<Self> {
        let is_stdin_tty = io::stdin().is_terminal();
        let is_stdout_tty = io::stdout().is_terminal();
        let is_stderr_tty = io::stderr().is_terminal();
        let color_enabled = ColorChoice::resolve(cli.no_color, is_stdout_tty).enabled();

        Ok(Self {
            stdin: StdinBackend::System(BufReader::new(io::stdin())),
            stdout: StdoutBackend::System,
            stderr: StderrBackend::System,
            is_stdin_tty,
            is_stdout_tty,
            is_stderr_tty,
            color_enabled,
            no_pager: cli.no_pager,
            pager: None,
        })
    }

    /// Constructs a buffer-backed `IoStreams` for tests.
    ///
    /// All streams are treated as non-TTY, color is disabled, and the pager
    /// is suppressed. Captured output is accessible via
    /// [`Self::stdout_as_string`] / [`Self::stderr_as_string`].
    #[must_use]
    pub fn test() -> Self {
        Self {
            stdin: StdinBackend::Buffer(Cursor::new(Vec::new())),
            stdout: StdoutBackend::Buffer(Arc::new(Mutex::new(Vec::new()))),
            stderr: StderrBackend::Buffer(Arc::new(Mutex::new(Vec::new()))),
            is_stdin_tty: false,
            is_stdout_tty: false,
            is_stderr_tty: false,
            color_enabled: false,
            no_pager: true,
            pager: None,
        }
    }

    /// Returns a writer bound to the active stdout backend (system, pager
    /// pipe, or in-memory buffer).
    pub fn stdout(&mut self) -> Box<dyn Write + '_> {
        match &mut self.stdout {
            StdoutBackend::System => Box::new(io::stdout().lock()),
            StdoutBackend::Pager(w) => Box::new(PagerWriter { inner: w.as_mut() }),
            StdoutBackend::Buffer(buf) => Box::new(BufferWriter { buf: buf.clone() }),
        }
    }

    /// Returns a writer bound to the active stderr backend.
    pub fn stderr(&mut self) -> Box<dyn Write + '_> {
        match &mut self.stderr {
            StderrBackend::System => Box::new(io::stderr().lock()),
            StderrBackend::Buffer(buf) => Box::new(BufferWriter { buf: buf.clone() }),
        }
    }

    /// Returns a reader bound to the active stdin backend.
    pub fn stdin(&mut self) -> &mut dyn BufRead {
        match &mut self.stdin {
            StdinBackend::System(r) => r,
            StdinBackend::Buffer(c) => c,
        }
    }

    /// Returns whether stdout is attached to a terminal.
    #[must_use]
    pub fn is_stdout_tty(&self) -> bool {
        self.is_stdout_tty
    }

    /// Returns whether stderr is attached to a terminal.
    #[must_use]
    pub fn is_stderr_tty(&self) -> bool {
        self.is_stderr_tty
    }

    /// Returns whether stdin is attached to a terminal.
    #[must_use]
    pub fn is_stdin_tty(&self) -> bool {
        self.is_stdin_tty
    }

    /// Returns whether color output is enabled for this IoStreams.
    #[must_use]
    pub fn color_enabled(&self) -> bool {
        self.color_enabled
    }

    /// Returns whether the pager has been explicitly disabled (either via
    /// `--no-pager`, `ATL_NO_PAGER`, or because this is a test instance).
    #[must_use]
    pub fn pager_disabled(&self) -> bool {
        self.no_pager
    }

    /// Starts the pager if stdout is a TTY and the pager is not disabled.
    ///
    /// Replaces `self.stdout` with the pipe to the pager's stdin. Calling
    /// this a second time while a pager is already running is a no-op.
    pub fn start_pager(&mut self) -> Result<()> {
        if self.pager.is_some() {
            return Ok(());
        }
        if self.no_pager || !self.is_stdout_tty {
            return Ok(());
        }
        // Only real system stdout should be replaced by the pager. Test /
        // buffer-backed streams must remain untouched.
        if !matches!(self.stdout, StdoutBackend::System) {
            return Ok(());
        }
        let Some(mut proc) = spawn_pager(self.no_pager)? else {
            return Ok(());
        };
        let Some(pipe) = proc.take_stdin() else {
            return Ok(());
        };
        self.stdout = StdoutBackend::Pager(pipe);
        self.pager = Some(proc);
        Ok(())
    }

    /// Closes the pager pipe and waits for the pager to exit.
    ///
    /// Idempotent — calling multiple times (or after the pager was never
    /// started) is a no-op.
    pub fn stop_pager(&mut self) -> Result<()> {
        // Swap the stdout back to the system so the pager's pipe is dropped,
        // sending EOF to the pager.
        if matches!(self.stdout, StdoutBackend::Pager(_)) {
            self.stdout = StdoutBackend::System;
        }
        if let Some(proc) = self.pager.take() {
            proc.wait()?;
        }
        Ok(())
    }

    /// Captures the stdout buffer as a string. Only valid on instances built
    /// with [`IoStreams::test`]; panics otherwise.
    #[must_use]
    pub fn stdout_as_string(&self) -> String {
        match &self.stdout {
            StdoutBackend::Buffer(buf) => {
                let guard = buf.lock().expect("stdout buffer mutex poisoned");
                String::from_utf8_lossy(&guard).into_owned()
            }
            _ => panic!("stdout_as_string called on a non-test IoStreams"),
        }
    }

    /// Captures the stderr buffer as a string. Only valid on instances built
    /// with [`IoStreams::test`]; panics otherwise.
    #[must_use]
    pub fn stderr_as_string(&self) -> String {
        match &self.stderr {
            StderrBackend::Buffer(buf) => {
                let guard = buf.lock().expect("stderr buffer mutex poisoned");
                String::from_utf8_lossy(&guard).into_owned()
            }
            _ => panic!("stderr_as_string called on a non-test IoStreams"),
        }
    }
}

impl Drop for IoStreams {
    fn drop(&mut self) {
        let _ = self.stop_pager();
    }
}

/// Thin writer that forwards to a `&mut dyn Write` borrowed from the pager
/// backend — needed so `stdout()` can return an owning `Box<dyn Write + '_>`.
struct PagerWriter<'a> {
    inner: &'a mut (dyn Write + Send),
}

impl Write for PagerWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Writer that appends to a shared `Vec<u8>` guarded by a mutex. Used by the
/// test backend so captured output remains accessible after the returned
/// writer is dropped.
struct BufferWriter {
    buf: Arc<Mutex<Vec<u8>>>,
}

impl Write for BufferWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut guard = self
            .buf
            .lock()
            .map_err(|_| io::Error::other("buffer mutex poisoned"))?;
        guard.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::env_lock;

    fn clear_color_env() {
        // SAFETY: tests using this helper hold `env_lock()`.
        unsafe {
            std::env::remove_var("NO_COLOR");
            std::env::remove_var("CLICOLOR_FORCE");
        }
    }

    #[test]
    fn color_resolve_no_color_flag_wins() {
        let _g = env_lock();
        clear_color_env();
        assert_eq!(ColorChoice::resolve(true, true), ColorChoice::Never);
    }

    #[test]
    fn color_resolve_no_color_env_wins() {
        let _g = env_lock();
        clear_color_env();
        // SAFETY: serialized by env_lock().
        unsafe { std::env::set_var("NO_COLOR", "1") };
        assert_eq!(ColorChoice::resolve(false, true), ColorChoice::Never);
        clear_color_env();
    }

    #[test]
    fn color_resolve_clicolor_force_always() {
        let _g = env_lock();
        clear_color_env();
        // SAFETY: serialized by env_lock().
        unsafe { std::env::set_var("CLICOLOR_FORCE", "1") };
        assert_eq!(ColorChoice::resolve(false, false), ColorChoice::Always);
        clear_color_env();
    }

    #[test]
    fn color_resolve_tty_is_auto() {
        let _g = env_lock();
        clear_color_env();
        assert_eq!(ColorChoice::resolve(false, true), ColorChoice::Auto);
    }

    #[test]
    fn color_resolve_non_tty_is_never() {
        let _g = env_lock();
        clear_color_env();
        assert_eq!(ColorChoice::resolve(false, false), ColorChoice::Never);
    }

    #[test]
    fn test_streams_capture_stdout() {
        let mut io = IoStreams::test();
        {
            let mut out = io.stdout();
            writeln!(out, "hello world").unwrap();
        }
        assert_eq!(io.stdout_as_string(), "hello world\n");
    }

    #[test]
    fn test_streams_capture_stderr() {
        let mut io = IoStreams::test();
        {
            let mut err = io.stderr();
            writeln!(err, "boom").unwrap();
        }
        assert_eq!(io.stderr_as_string(), "boom\n");
    }

    #[test]
    fn test_streams_start_pager_is_noop() {
        let mut io = IoStreams::test();
        io.start_pager().unwrap();
        assert!(io.pager.is_none());
        {
            let mut out = io.stdout();
            writeln!(out, "still captured").unwrap();
        }
        assert_eq!(io.stdout_as_string(), "still captured\n");
    }

    #[test]
    fn no_pager_flag_disables_pager() {
        // Simulates a system IoStreams that has --no-pager set even though
        // stdout would otherwise be a TTY. We construct the struct directly
        // because `IoStreams::system` requires a parsed CLI.
        let mut io = IoStreams {
            stdin: StdinBackend::Buffer(Cursor::new(Vec::new())),
            stdout: StdoutBackend::System,
            stderr: StderrBackend::System,
            is_stdin_tty: true,
            is_stdout_tty: true,
            is_stderr_tty: true,
            color_enabled: false,
            no_pager: true,
            pager: None,
        };
        io.start_pager().unwrap();
        assert!(
            io.pager.is_none(),
            "pager must not start when no_pager=true"
        );
        assert!(matches!(io.stdout, StdoutBackend::System));
    }
}
