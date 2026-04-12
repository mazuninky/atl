use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use assert_cmd::cargo::cargo_bin;

/// Maximum time a single `atl` invocation may run before the runner kills
/// it. Real contract calls reach the local Prism mock in milliseconds; if
/// the binary hangs we want a clear failure rather than a stalled suite.
const ATL_TIMEOUT: Duration = Duration::from_secs(60);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Runner for invoking the `atl` binary with a specific config file.
pub struct AtlRunner {
    config_path: PathBuf,
}

/// Captured result of an `atl` invocation.
pub struct AtlResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl AtlRunner {
    /// Create a new runner that uses the given config file for all invocations.
    pub fn new(config_path: &Path) -> Self {
        Self {
            config_path: config_path.to_path_buf(),
        }
    }

    /// Run `atl` with the given arguments, prepending `--config <path>` and
    /// `--profile test` (the latter prevents an `ATL_PROFILE` env var in the
    /// developer's shell from leaking into the test).
    ///
    /// Returns the full captured result including exit code, stdout, and
    /// stderr. If the binary does not exit within `ATL_TIMEOUT`, the child
    /// is killed and the runner panics with a clear timeout message.
    pub fn run(&self, args: &[&str]) -> AtlResult {
        let bin = cargo_bin("atl");

        // Force --profile test unless the caller is explicitly testing
        // profile resolution itself. This isolates tests from a developer
        // shell that has `ATL_PROFILE` exported.
        let caller_overrides_profile = args.iter().any(|a| *a == "--profile" || *a == "-p");
        let mut cmd = Command::new(&bin);
        cmd.arg("--config").arg(&self.config_path);
        if !caller_overrides_profile {
            cmd.arg("--profile").arg("test");
        }
        // Supply the token via env var (highest priority in the auth
        // resolution chain) instead of the deprecated `api_token` TOML
        // field, which triggers a tracing::warn that pollutes stdout.
        cmd.env("ATL_API_TOKEN", "test-token");
        let mut child = cmd
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("Failed to spawn atl at {}: {e}", bin.display()));

        let mut stdout_pipe = child.stdout.take().expect("piped stdout");
        let mut stderr_pipe = child.stderr.take().expect("piped stderr");
        let stdout_handle = thread::spawn(move || {
            let mut buf = Vec::new();
            stdout_pipe.read_to_end(&mut buf).ok();
            buf
        });
        let stderr_handle = thread::spawn(move || {
            let mut buf = Vec::new();
            stderr_pipe.read_to_end(&mut buf).ok();
            buf
        });

        // Bounded wait via try_wait polling.
        let pid = child.id();
        let deadline = Instant::now() + ATL_TIMEOUT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        panic!(
                            "atl (pid {pid}) did not exit within {ATL_TIMEOUT:?}; args: {args:?}"
                        );
                    }
                    thread::sleep(POLL_INTERVAL);
                }
                Err(e) => panic!("waiting on atl (pid {pid}) failed: {e}"),
            }
        };

        let stdout = stdout_handle.join().unwrap_or_default();
        let stderr = stderr_handle.join().unwrap_or_default();

        AtlResult {
            exit_code: status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        }
    }

    /// Run `atl` and return stdout on success.
    ///
    /// In contract tests we only care that atl's HTTP **request** conforms to
    /// the OpenAPI spec. Prism validates the request first; if it is malformed
    /// Prism returns `422`. If the request is valid Prism then tries to
    /// generate a mock response, which can fail with `500 SCHEMA_TOO_COMPLEX`
    /// on deeply recursive schemas (common in Atlassian specs). Both "Prism
    /// could not generate a response" and "Prism generated a response but
    /// atl failed to parse it" indicate the request itself was valid, so we
    /// treat them as success.
    ///
    /// # Panics
    ///
    /// Panics if the exit code is not 0 and the stderr does not match a
    /// known "request valid, response generation failed" pattern.
    pub fn run_ok(&self, args: &[&str]) -> String {
        let result = self.run(args);
        if result.exit_code == 0 {
            return result.stdout;
        }

        if is_prism_response_generation_error(&result.stderr) {
            return result.stdout;
        }

        panic!(
            "atl exited with code {} (expected 0 or a Prism response-generation error)\n\
             args: {:?}\nstderr:\n{}",
            result.exit_code, args, result.stderr
        );
    }

    /// Run `atl` and return stderr on expected failure.
    ///
    /// # Panics
    ///
    /// Panics if the exit code does not match `expected_code`, including both
    /// stdout and stderr in the panic message.
    pub fn run_err(&self, args: &[&str], expected_code: i32) -> String {
        let result = self.run(args);
        assert_eq!(
            result.exit_code, expected_code,
            "atl exited with code {} (expected {expected_code})\n\
             args: {args:?}\n\
             stdout:\n{}\nstderr:\n{}",
            result.exit_code, result.stdout, result.stderr
        );
        result.stderr
    }
}

/// Returns `true` if the stderr output looks like Prism failed to generate a
/// mock response but the request itself was valid.
///
/// Prism returns 500 with specific error codes when `json-schema-faker` can't
/// synthesize a response body for deeply recursive Atlassian schemas. We also
/// accept atl's JSON/parse errors because they imply a response *was*
/// generated but was malformed (still means the request reached Prism).
fn is_prism_response_generation_error(stderr: &str) -> bool {
    // A genuine request-validation failure from Prism looks like:
    //   `status":422 ... stoplight.io/prism/errors#UNPROCESSABLE_ENTITY`
    // If we see that, the test must fail — atl sent a bad request.
    if stderr.contains("UNPROCESSABLE_ENTITY") {
        return false;
    }
    // A 404 with `NO_PATH_MATCHED_ERROR` means atl sent a request to a
    // path or method that does not exist in the spec. That is exactly what
    // contract testing is supposed to catch, so it must NOT be treated as
    // a Prism mock-generation artefact.
    if stderr.contains("NO_PATH_MATCHED_ERROR") || stderr.contains("NO_METHOD_MATCHED_ERROR") {
        return false;
    }

    const PRISM_MARKERS: &[&str] = &[
        "SCHEMA_TOO_COMPLEX",
        "Schema too complex",
        "Prop not found",
        "json-schema-faker",
        // Prism returns 302 for endpoints like Confluence attachment download
        // without a Location header because the spec does not define one. In
        // real Confluence the 302 points at a CDN URL that reqwest follows.
        "API error: 302",
        // Prism returned a spec-defined non-2xx response because the spec
        // only lists error status codes. The request itself was valid (no
        // 422 / NO_PATH_MATCHED checked above).
        "API error: 400",
        "API error: 401",
    ];
    const ATL_PARSE_MARKERS: &[&str] = &[
        "JSON error:",
        "missing field",
        "invalid type",
        // atl tried to parse an empty body that Prism produced for a 2xx
        // response with no example.
        "EOF while parsing",
        "error decoding response body",
    ];

    PRISM_MARKERS.iter().any(|m| stderr.contains(m))
        || ATL_PARSE_MARKERS.iter().any(|m| stderr.contains(m))
}
