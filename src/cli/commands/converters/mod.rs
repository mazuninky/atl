//! Format converters that bridge user-facing markdown to Atlassian formats.
//!
//! Each child module is a pure-logic conversion between one source format and
//! one target format. Modules are siblings — there is no shared trait or
//! plugin system; converters are independent because they have very different
//! intermediate representations and error surfaces.
//!
//! Today there are six converters:
//!
//! - [`md_to_storage`] — markdown (with MyST-style directive extensions) →
//!   Confluence storage XHTML.
//! - [`storage_to_md`] — Confluence storage XHTML → markdown (the inverse of
//!   the above).
//! - [`md_to_adf`] — markdown (with MyST-style directive extensions) →
//!   Atlassian Document Format JSON (used by Confluence Cloud and Jira Cloud).
//! - [`adf_to_md`] — Atlassian Document Format JSON → markdown (the inverse of
//!   the above).
//! - [`md_to_wiki`] — markdown (with MyST-style directive extensions) → Jira
//!   wiki text (the legacy markup still accepted by the Jira REST API).
//! - [`wiki_to_md`] — Jira wiki text → markdown (the inverse of the above).
//!
//! Converters never perform IO and never log. They are called from command
//! handlers that read user input (via `read_body_arg`) and ship the result to
//! the appropriate Atlassian API client.

pub mod adf_to_md;
pub mod body_content;
pub mod md_to_adf;
pub mod md_to_storage;
pub mod md_to_wiki;
pub mod storage_to_md;
pub mod wiki_to_md;
