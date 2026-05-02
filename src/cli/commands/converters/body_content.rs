//! Output of body conversions for Confluence and Jira write paths.
//!
//! [`BodyContent`] (Confluence) and [`JiraBodyContent`] (Jira) are the bridge
//! types between user-facing input formats (markdown, storage XHTML, wiki text,
//! ADF) and the wire-format payloads produced by the respective Atlassian
//! clients. After conversion, command handlers hand the appropriate enum to
//! their client, which renders the matching JSON.
//!
//! ## Why two enums?
//!
//! The two services route ADF through different API surfaces, so the variants
//! and the routing decisions they imply are different:
//!
//! - Confluence: ADF and storage both live in v2; `BodyContent` only encodes
//!   the body shape, and the client picks `body.storage.value` vs
//!   `body.atlas_doc_format.value` accordingly.
//! - Jira: ADF requires the v3 API (Cloud only) while wiki text uses v2.
//!   `JiraBodyContent` therefore drives both the body shape AND which API
//!   version the client must call.
//!
//! Sharing a single enum would conflate these very different routing models.
//!
//! Lives in the `converters` module so neither the `cli::commands` layer nor
//! the `client` layer has to import from the other.

use serde_json::Value;

/// The result of normalising a user-supplied body into a representation the
/// Confluence v2 API understands.
///
/// - [`BodyContent::Storage`] holds Confluence storage XHTML — the payload
///   becomes `body.storage.value` with `representation: "storage"`.
/// - [`BodyContent::Adf`] holds an Atlassian Document Format JSON value —
///   the payload becomes `body.atlas_doc_format.value` (stringified JSON)
///   with `representation: "atlas_doc_format"`.
#[derive(Debug, Clone)]
pub enum BodyContent {
    /// Storage XHTML — the historical Confluence wire format.
    Storage(String),
    /// Atlassian Document Format JSON — the native Cloud format.
    Adf(Value),
}

/// Body payload variants for Jira issue / comment writes.
///
/// The variant chosen by the caller determines which API path the client
/// takes — [`JiraBodyContent::Wiki`] goes through v2 (wiki text), while
/// [`JiraBodyContent::Adf`] goes through v3 (ADF object). The Jira v3 API is
/// only available on Cloud; ADF input on Data Center / Server is rejected at
/// the command-handler layer with a typed [`crate::error::Error::Config`].
#[derive(Debug, Clone)]
pub enum JiraBodyContent {
    /// Jira wiki text — sent as a plain string in v2 issue/comment payloads.
    Wiki(String),
    /// ADF JSON — sent as the JSON object directly in v3 issue/comment payloads.
    Adf(Value),
}
