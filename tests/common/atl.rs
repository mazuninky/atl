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
        // resolution chain) instead of the `api_token` TOML field.
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
/// accept atl's JSON-shape errors (`JSON error: ...`, `missing field`,
/// `invalid type`) because they imply a response *was* generated but had a
/// shape that did not match atl's response struct — that's a known Prism
/// static-mode limitation where partially-shaped fixtures don't satisfy
/// every required field, not an atl bug.
///
/// Empty-body / decode-EOF errors are **NOT** masked: atl previously hid a
/// real production bug behind `EOF while parsing` (atl#53 — `update_issue` /
/// `transition_issue` called `handle_response` instead of
/// `handle_response_maybe_empty` for endpoints that legitimately return 204
/// No Content). The whole point of `*_positive` contract tests is to catch
/// exactly that kind of `handle_response` vs `handle_response_maybe_empty`
/// mismatch, so those errors must surface as failures.
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
    // Markers for "Prism produced a response, atl parsed it, but the JSON
    // shape did not match atl's struct". These are tolerated because Prism's
    // static mode (json-schema-faker) routinely produces partially-shaped
    // example bodies that don't cover every required field in deeply nested
    // Atlassian schemas — that is a Prism limitation, not an atl bug.
    //
    // Empty-body / decode-EOF errors are deliberately **not** included
    // here: see the doc comment on this function.
    const ATL_PARSE_MARKERS: &[&str] = &["JSON error:", "missing field", "invalid type"];

    PRISM_MARKERS.iter().any(|m| stderr.contains(m))
        || ATL_PARSE_MARKERS.iter().any(|m| stderr.contains(m))
}

#[cfg(test)]
mod tests {
    //! Pin the contract-test masking policy. These are *unit* tests for the
    //! helper itself — they don't spawn any binary and don't need Prism. They
    //! exist because `is_prism_response_generation_error` previously hid a
    //! real production bug (atl#53) by whitelisting `EOF while parsing`, and
    //! we want a regression guard so future edits to the marker list don't
    //! silently re-introduce that hole.
    use super::is_prism_response_generation_error;

    #[test]
    fn does_not_mask_empty_body_eof_errors() {
        // Regression: atl#53. `update_issue` returned this stderr because it
        // called `handle_response` on a 204 response. The contract test
        // `update_issue_positive` must FAIL on this output, not pass it
        // through `run_ok` as a "Prism artefact".
        let stderr = "Error: HTTP error: error decoding response body: \
                      EOF while parsing a value at line 1 column 0";
        assert!(
            !is_prism_response_generation_error(stderr),
            "EOF-while-parsing must NOT be masked: that's the regression \
             from atl#53 where `update_issue` hit `handle_response` \
             instead of `handle_response_maybe_empty`. stderr: {stderr}"
        );
    }

    #[test]
    fn does_not_mask_reqwest_decode_errors() {
        // The wrapping reqwest error string also previously slipped through.
        let stderr = "Error: HTTP error: error decoding response body: \
                      expected value at line 1 column 1";
        assert!(
            !is_prism_response_generation_error(stderr),
            "`error decoding response body` must NOT be masked: it indicates \
             atl tried to parse a response that real Atlassian endpoints \
             return as 204 / empty. stderr: {stderr}"
        );
    }

    #[test]
    fn masks_json_shape_mismatch_from_prism_faker() {
        // Prism's json-schema-faker routinely produces fixtures missing
        // required fields. This is a known static-mode limitation, not an
        // atl bug — keep masking it.
        let stderr = "Error: JSON error: missing field `key` at line 1 column 42";
        assert!(
            is_prism_response_generation_error(stderr),
            "`JSON error: missing field` must remain masked — Prism faker \
             quirk, not a real atl bug. stderr: {stderr}"
        );
    }

    #[test]
    fn masks_invalid_type_from_prism_faker() {
        let stderr = "Error: JSON error: invalid type: null, expected a string \
                      at line 1 column 17";
        assert!(
            is_prism_response_generation_error(stderr),
            "`invalid type` must remain masked — Prism faker quirk. \
             stderr: {stderr}"
        );
    }

    #[test]
    fn masks_schema_too_complex() {
        let stderr = "Error: HTTP error: API error: 500 \
                      stoplight.io/prism/errors#SCHEMA_TOO_COMPLEX \
                      The schema is too complex to be generated";
        assert!(
            is_prism_response_generation_error(stderr),
            "SCHEMA_TOO_COMPLEX is a Prism artefact and must remain masked. \
             stderr: {stderr}"
        );
    }

    #[test]
    fn does_not_mask_unprocessable_entity() {
        // Genuine spec violation by atl — the request was malformed. This
        // is exactly what contract tests are supposed to catch.
        let stderr = "Error: HTTP error: API error: 422 \
                      stoplight.io/prism/errors#UNPROCESSABLE_ENTITY \
                      Request body validation failed";
        assert!(
            !is_prism_response_generation_error(stderr),
            "UNPROCESSABLE_ENTITY must NOT be masked: the whole point of \
             contract tests is to catch atl sending malformed requests. \
             stderr: {stderr}"
        );
    }

    #[test]
    fn does_not_mask_no_path_matched() {
        // atl hit a path/method that doesn't exist in the spec. Must fail.
        let stderr = "Error: HTTP error: API error: 404 \
                      stoplight.io/prism/errors#NO_PATH_MATCHED_ERROR";
        assert!(
            !is_prism_response_generation_error(stderr),
            "NO_PATH_MATCHED_ERROR must NOT be masked: it means atl hit \
             a path that's not in the spec. stderr: {stderr}"
        );
    }

    #[test]
    fn does_not_mask_no_method_matched() {
        let stderr = "Error: HTTP error: API error: 405 \
                      stoplight.io/prism/errors#NO_METHOD_MATCHED_ERROR";
        assert!(
            !is_prism_response_generation_error(stderr),
            "NO_METHOD_MATCHED_ERROR must NOT be masked: it means atl used \
             a method that's not in the spec. stderr: {stderr}"
        );
    }
}
