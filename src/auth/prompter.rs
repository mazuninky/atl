//! Interactive prompting abstraction used by `atl auth login`.
//!
//! The [`Prompter`] trait lets the login flow be unit-tested with a scripted
//! [`MockPrompter`] instead of driving `inquire` under a real TTY.
//!
//! Production uses [`InquirePrompter`], which wraps the `inquire` crate.
//! When stdin/stdout are not TTYs, `inquire` returns `InquireError::NotTTY`
//! — we convert that to a clear "interactive login requires a TTY" message
//! so CI / pipe users get an actionable error instead of a panic.

use std::collections::VecDeque;
use std::sync::Mutex;

use anyhow::{Result, anyhow};

/// Interactive prompting primitives used by the login wizard.
///
/// Implementors do not need to be `Send + Sync` — prompts happen on a single
/// thread inside the command handler.
pub trait Prompter {
    /// Reads a single line of free-form text.
    fn text(&self, msg: &str, default: Option<&str>) -> Result<String>;

    /// Reads a secret (no echo). The implementation is expected to disable
    /// terminal echo; [`MockPrompter`] simply returns the scripted value.
    fn password(&self, msg: &str) -> Result<String>;

    /// Asks the user to pick one of `options`, returning the zero-based
    /// index of their choice.
    fn select(&self, msg: &str, options: &[&str]) -> Result<usize>;

    /// Asks a yes/no question. `default` is the value used when the user
    /// presses Enter without typing anything.
    fn confirm(&self, msg: &str, default: bool) -> Result<bool>;
}

/// Production [`Prompter`] implementation backed by the `inquire` crate.
#[derive(Debug, Default, Clone, Copy)]
pub struct InquirePrompter;

fn map_inquire_err(err: inquire::InquireError) -> anyhow::Error {
    match err {
        inquire::InquireError::NotTTY => anyhow!(
            "interactive login requires a TTY; pipe the token via `--with-token` or run in an interactive shell"
        ),
        inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted => {
            anyhow!("login cancelled")
        }
        other => anyhow!("prompt failed: {other}"),
    }
}

impl Prompter for InquirePrompter {
    fn text(&self, msg: &str, default: Option<&str>) -> Result<String> {
        let mut prompt = inquire::Text::new(msg);
        if let Some(d) = default {
            prompt = prompt.with_default(d);
        }
        prompt.prompt().map_err(map_inquire_err)
    }

    fn password(&self, msg: &str) -> Result<String> {
        inquire::Password::new(msg)
            // No confirmation prompt — the user already has the token.
            .without_confirmation()
            .with_display_mode(inquire::PasswordDisplayMode::Masked)
            .prompt()
            .map_err(map_inquire_err)
    }

    fn select(&self, msg: &str, options: &[&str]) -> Result<usize> {
        if options.is_empty() {
            return Err(anyhow!("select prompt requires at least one option"));
        }
        let owned: Vec<String> = options.iter().map(|s| (*s).to_string()).collect();
        let choice = inquire::Select::new(msg, owned.clone())
            .with_scorer(&|input, _, string_value, _| {
                if string_value.to_lowercase().contains(&input.to_lowercase()) {
                    Some(0)
                } else {
                    None
                }
            })
            .prompt()
            .map_err(map_inquire_err)?;
        owned
            .iter()
            .position(|s| s == &choice)
            .ok_or_else(|| anyhow!("selected option not found in list"))
    }

    fn confirm(&self, msg: &str, default: bool) -> Result<bool> {
        inquire::Confirm::new(msg)
            .with_default(default)
            .prompt()
            .map_err(map_inquire_err)
    }
}

/// A scripted response for [`MockPrompter`].
#[derive(Debug, Clone)]
pub enum MockResponse {
    /// Return `String` from the next `text` call.
    Text(String),
    /// Return `String` from the next `password` call.
    Password(String),
    /// Return the given index from the next `select` call.
    Select(usize),
    /// Return the given bool from the next `confirm` call.
    Confirm(bool),
}

/// Test prompter backed by a FIFO of scripted responses.
///
/// Each `Prompter` method pops the next response from the queue. Calls that
/// pop a mismatched response type (e.g. `text()` finding a `MockResponse::Password`)
/// return an error — this makes miswritten tests fail loudly instead of
/// silently accepting wrong data.
#[derive(Debug, Default)]
pub struct MockPrompter {
    responses: Mutex<VecDeque<MockResponse>>,
}

impl MockPrompter {
    /// Creates a prompter with the given scripted responses, consumed in
    /// order.
    #[must_use]
    pub fn new(responses: Vec<MockResponse>) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from(responses)),
        }
    }

    /// Returns the number of responses still queued.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.responses.lock().map(|g| g.len()).unwrap_or_default()
    }

    fn pop(&self) -> Result<MockResponse> {
        self.responses
            .lock()
            .map_err(|_| anyhow!("mock prompter mutex poisoned"))?
            .pop_front()
            .ok_or_else(|| anyhow!("mock prompter: no more scripted responses"))
    }
}

impl Prompter for MockPrompter {
    fn text(&self, _msg: &str, _default: Option<&str>) -> Result<String> {
        match self.pop()? {
            MockResponse::Text(s) => Ok(s),
            other => Err(anyhow!("mock prompter: expected Text, got {other:?}")),
        }
    }

    fn password(&self, _msg: &str) -> Result<String> {
        match self.pop()? {
            MockResponse::Password(s) => Ok(s),
            other => Err(anyhow!("mock prompter: expected Password, got {other:?}")),
        }
    }

    fn select(&self, _msg: &str, options: &[&str]) -> Result<usize> {
        match self.pop()? {
            MockResponse::Select(i) => {
                if i >= options.len() {
                    Err(anyhow!(
                        "mock prompter: select index {i} out of range (len={})",
                        options.len()
                    ))
                } else {
                    Ok(i)
                }
            }
            other => Err(anyhow!("mock prompter: expected Select, got {other:?}")),
        }
    }

    fn confirm(&self, _msg: &str, _default: bool) -> Result<bool> {
        match self.pop()? {
            MockResponse::Confirm(b) => Ok(b),
            other => Err(anyhow!("mock prompter: expected Confirm, got {other:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_returns_scripted_responses_in_order() {
        let prompter = MockPrompter::new(vec![
            MockResponse::Text("acme.atlassian.net".into()),
            MockResponse::Text("alice@acme.com".into()),
            MockResponse::Password("secret".into()),
        ]);
        assert_eq!(
            prompter.text("domain?", None).unwrap(),
            "acme.atlassian.net"
        );
        assert_eq!(prompter.text("email?", None).unwrap(), "alice@acme.com");
        assert_eq!(prompter.password("token?").unwrap(), "secret");
        assert_eq!(prompter.remaining(), 0);
    }

    #[test]
    fn mock_select_in_range() {
        let prompter = MockPrompter::new(vec![MockResponse::Select(1)]);
        let choice = prompter.select("pick", &["a", "b", "c"]).unwrap();
        assert_eq!(choice, 1);
    }

    #[test]
    fn mock_select_out_of_range_errors() {
        let prompter = MockPrompter::new(vec![MockResponse::Select(5)]);
        assert!(prompter.select("pick", &["a", "b"]).is_err());
    }

    #[test]
    fn mock_confirm_returns_value() {
        let prompter = MockPrompter::new(vec![MockResponse::Confirm(true)]);
        assert!(prompter.confirm("sure?", false).unwrap());
    }

    #[test]
    fn mock_text_errors_on_wrong_type() {
        let prompter = MockPrompter::new(vec![MockResponse::Password("x".into())]);
        assert!(prompter.text("domain?", None).is_err());
    }

    #[test]
    fn mock_empty_queue_errors() {
        let prompter = MockPrompter::new(vec![]);
        assert!(prompter.text("domain?", None).is_err());
    }
}
