use std::io::Read;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, ChildStderr, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// A managed Prism mock server process that validates HTTP requests against an OpenAPI spec.
///
/// The server is automatically killed when this value is dropped.
pub struct PrismServer {
    process: Child,
    port: u16,
    base_url: String,
    /// Captured stderr from the Prism child, populated by a background reader
    /// thread. Used to attach process output to startup-failure panics so a
    /// crashed Prism doesn't masquerade as a generic timeout.
    stderr_buf: Arc<Mutex<Vec<u8>>>,
}

impl PrismServer {
    /// Start a Prism mock server for the given OpenAPI spec file.
    ///
    /// Finds a free port, spawns the Prism CLI process, and waits until it is ready
    /// to accept connections.
    ///
    /// # Panics
    ///
    /// Panics if the spec file does not exist, if the Prism binary cannot be
    /// spawned, or if the server fails to become ready within the timeout
    /// period.
    pub fn start(spec_path: &str) -> Self {
        let spec = Path::new(spec_path);
        assert!(
            spec.exists(),
            "OpenAPI spec file not found: {spec_path}. \
             Make sure the spec file exists at the specified path."
        );

        let port = find_free_port();
        let base_url = format!("http://127.0.0.1:{port}");

        // `ATL_PRISM_BIN` overrides the binary name for non-`prism` setups (e.g. a Docker wrapper).
        let prism_bin = std::env::var("ATL_PRISM_BIN").unwrap_or_else(|_| "prism".to_string());

        // `--dynamic` uses json-schema-faker, which crashes on Confluence's deeply recursive
        // schemas; static mode returns spec examples instead.
        let mut process = Command::new(&prism_bin)
            .args([
                "mock",
                spec_path,
                "--port",
                &port.to_string(),
                "--host",
                "127.0.0.1",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| {
                panic!(
                    "Failed to spawn Prism CLI ({prism_bin}): {e}. \
                     Install with `npm install -g @stoplight/prism-cli`, \
                     or grab the standalone binary from \
                     https://github.com/stoplightio/prism/releases, \
                     or set ATL_PRISM_BIN to an existing executable."
                )
            });

        let stderr_buf = Arc::new(Mutex::new(Vec::new()));
        if let Some(stderr) = process.stderr.take() {
            spawn_stderr_reader(stderr, Arc::clone(&stderr_buf));
        }

        let mut server = Self {
            process,
            port,
            base_url,
            stderr_buf,
        };
        server.wait_ready();
        server
    }

    /// Returns the base URL of the running Prism server (e.g. `http://127.0.0.1:12345`).
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Returns the port the Prism server is listening on.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Poll the server until it accepts TCP connections.
    ///
    /// Prism startup can take 10-30 seconds because the Node.js runtime parses
    /// large OpenAPI specs. Retries up to 300 times with a 300ms sleep between
    /// attempts (90 seconds total). If the Prism child
    /// exits before becoming ready, we panic immediately with the captured
    /// stderr instead of waiting for the full timeout.
    fn wait_ready(&mut self) {
        let addr = format!("127.0.0.1:{}", self.port);
        let sleep = Duration::from_millis(300);
        let connect_timeout = Duration::from_millis(200);
        const MAX_ATTEMPTS: u32 = 300;

        for attempt in 1..=MAX_ATTEMPTS {
            if std::net::TcpStream::connect_timeout(
                &addr.parse().expect("valid socket address"),
                connect_timeout,
            )
            .is_ok()
            {
                return;
            }
            if let Some(status) = self
                .process
                .try_wait()
                .expect("failed to poll Prism child status")
            {
                panic!(
                    "Prism child exited before becoming ready (status: {status}).\n\
                     Captured stderr:\n{}",
                    self.captured_stderr()
                );
            }
            if attempt < MAX_ATTEMPTS {
                thread::sleep(sleep);
            }
        }

        panic!(
            "Prism server on port {} failed to become ready after {MAX_ATTEMPTS} attempts (90s total).\n\
             Captured stderr:\n{}",
            self.port,
            self.captured_stderr()
        );
    }

    fn captured_stderr(&self) -> String {
        let buf = self.stderr_buf.lock().expect("stderr buffer poisoned");
        String::from_utf8_lossy(&buf).into_owned()
    }
}

fn spawn_stderr_reader(mut stderr: ChildStderr, sink: Arc<Mutex<Vec<u8>>>) {
    thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        loop {
            match stderr.read(&mut chunk) {
                Ok(0) => return,
                Ok(n) => {
                    if let Ok(mut buf) = sink.lock() {
                        buf.extend_from_slice(&chunk[..n]);
                    }
                }
                Err(_) => return,
            }
        }
    });
}

impl Drop for PrismServer {
    fn drop(&mut self) {
        self.process.kill().ok();
        self.process.wait().ok();
    }
}

/// Bind to port 0 to let the OS assign a free port, then return it.
fn find_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind to ephemeral port");
    listener
        .local_addr()
        .expect("failed to get local address")
        .port()
}
