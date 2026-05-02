use camino::Utf8PathBuf;
use clap::{Args, Subcommand};

use super::JiraInputFormat;

#[derive(Debug, Args)]
pub struct JiraSearchArgs {
    /// JQL query (combined with filter flags via AND)
    pub jql: Option<String>,

    /// Max results per page
    #[arg(long, short, default_value = "50")]
    pub limit: u32,

    /// Fetch all results (auto-paginate)
    #[arg(long)]
    pub all: bool,

    /// Fields to return (comma-separated)
    #[arg(long, short, default_value = "key,summary,status,assignee,priority")]
    pub fields: String,

    /// Filter by status name (e.g. "Open", "In Progress")
    #[arg(long)]
    pub status: Option<String>,

    /// Filter by priority name
    #[arg(long)]
    pub priority: Option<String>,

    /// Filter by assignee (account ID or "currentUser()")
    #[arg(long)]
    pub assignee: Option<String>,

    /// Filter by reporter
    #[arg(long)]
    pub reporter: Option<String>,

    /// Filter by issue type (e.g. Bug, Task, Story)
    #[arg(long, value_name = "TYPE")]
    pub r#type: Option<String>,

    /// Filter by label
    #[arg(long)]
    pub label: Option<String>,

    /// Filter by component
    #[arg(long)]
    pub component: Option<String>,

    /// Filter by resolution
    #[arg(long)]
    pub resolution: Option<String>,

    /// Filter: created on or after date (YYYY-MM-DD)
    #[arg(long)]
    pub created: Option<String>,

    /// Filter: created after date (YYYY-MM-DD)
    #[arg(long)]
    pub created_after: Option<String>,

    /// Filter: updated on or after date (YYYY-MM-DD)
    #[arg(long)]
    pub updated: Option<String>,

    /// Filter: updated after date (YYYY-MM-DD)
    #[arg(long)]
    pub updated_after: Option<String>,

    /// Only issues you are watching
    #[arg(long)]
    pub watching: bool,

    /// Order results by field (e.g. "created", "priority")
    #[arg(long)]
    pub order_by: Option<String>,

    /// Reverse sort order (use with --order-by)
    #[arg(long)]
    pub reverse: bool,
}

#[derive(Debug, Args)]
pub struct JiraIssueKeyArgs {
    /// Issue key (e.g. PROJ-123)
    pub key: String,
}

#[derive(Debug, Args)]
pub struct JiraViewArgs {
    /// Issue key (e.g. PROJ-123)
    pub key: String,

    /// Open the issue in a browser instead of printing
    #[arg(long)]
    pub web: bool,
}

#[derive(Debug, Args)]
pub struct JiraDeleteArgs {
    /// Issue key (e.g. PROJ-123)
    pub key: String,

    /// Also delete subtasks
    #[arg(long)]
    pub delete_subtasks: bool,
}

#[derive(Debug, Args)]
pub struct JiraCreateArgs {
    /// Project key
    #[arg(long)]
    pub project: String,

    /// Issue type (e.g. Task, Bug, Story)
    #[arg(long, short = 't')]
    pub issue_type: String,

    /// Summary
    #[arg(long, short)]
    pub summary: String,

    /// Description
    #[arg(long, short)]
    pub description: Option<String>,

    /// Assignee account ID
    #[arg(long)]
    pub assignee: Option<String>,

    /// Priority name
    #[arg(long)]
    pub priority: Option<String>,

    /// Labels (comma-separated)
    #[arg(long)]
    pub labels: Option<String>,

    /// Parent issue key (for subtasks)
    #[arg(long)]
    pub parent: Option<String>,

    /// Fix version(s), comma-separated
    #[arg(long)]
    pub fix_version: Option<String>,

    /// Component(s), comma-separated
    #[arg(long)]
    pub component: Option<String>,

    /// Custom field (repeatable), e.g. --custom customfield_10001=value
    #[arg(long = "custom", value_name = "KEY=VALUE")]
    pub custom_fields: Vec<String>,

    /// Input format for the body
    #[arg(long, default_value = "wiki", value_enum)]
    pub input_format: JiraInputFormat,
}

#[derive(Debug, Args)]
pub struct JiraUpdateArgs {
    /// Issue key
    pub key: String,

    /// New summary
    #[arg(long, short)]
    pub summary: Option<String>,

    /// New description
    #[arg(long, short)]
    pub description: Option<String>,

    /// New assignee account ID
    #[arg(long)]
    pub assignee: Option<String>,

    /// New priority name
    #[arg(long)]
    pub priority: Option<String>,

    /// Labels (comma-separated, replaces existing)
    #[arg(long)]
    pub labels: Option<String>,

    /// Fix version(s), comma-separated
    #[arg(long)]
    pub fix_version: Option<String>,

    /// Component(s), comma-separated
    #[arg(long)]
    pub component: Option<String>,

    /// Custom field (repeatable), e.g. --custom customfield_10001=value
    #[arg(long = "custom", value_name = "KEY=VALUE")]
    pub custom_fields: Vec<String>,

    /// Input format for the body
    #[arg(long, default_value = "wiki", value_enum)]
    pub input_format: JiraInputFormat,
}

#[derive(Debug, Args)]
pub struct JiraMoveArgs {
    /// Issue key
    pub key: String,

    /// Transition ID
    #[arg(long, short)]
    pub transition: String,
}

#[derive(Debug, Args)]
pub struct JiraAssignArgs {
    /// Issue key
    pub key: String,

    /// Assignee account ID
    pub account_id: String,
}

#[derive(Debug, Args)]
pub struct JiraCommentArgs {
    /// Issue key
    pub key: String,

    /// Comment body. Use @file to read from file, or - for stdin
    pub body: String,

    /// Input format for the body
    #[arg(long, default_value = "wiki", value_enum)]
    pub input_format: JiraInputFormat,
}

#[derive(Debug, Args)]
pub struct JiraLinkArgs {
    /// Link type name (e.g. "Blocks", "Duplicates")
    #[arg(long, short = 't')]
    pub link_type: String,

    /// Inward issue key
    pub inward_key: String,

    /// Outward issue key
    pub outward_key: String,
}

#[derive(Debug, Args)]
pub struct JiraRemoteLinkAddArgs {
    /// Issue key
    pub key: String,

    /// URL to link
    pub url: String,

    /// Link title (defaults to the URL if omitted)
    #[arg(long, short)]
    pub title: Option<String>,
}

#[derive(Debug, Args)]
pub struct JiraCloneArgs {
    /// Issue key to clone
    pub key: String,

    /// Override the summary for the cloned issue
    #[arg(long, short)]
    pub summary: Option<String>,
}

#[derive(Debug, Args)]
pub struct JiraChangelogArgs {
    /// Issue key (e.g. PROJ-123)
    pub key: String,

    /// Max results per page
    #[arg(long, short, default_value = "100")]
    pub limit: u32,

    /// Start at index (for pagination)
    #[arg(long, default_value = "0")]
    pub start_at: u32,

    /// Fetch all results (auto-paginate)
    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct JiraAttachArgs {
    /// Issue key (e.g. PROJ-123)
    pub key: String,

    /// Path to the file to attach
    #[arg(long, short)]
    pub file: Utf8PathBuf,
}

#[derive(Debug, Args)]
pub struct JiraCommentGetArgs {
    /// Issue key (e.g. PROJ-123)
    pub key: String,

    /// Comment ID
    pub comment_id: String,
}

#[derive(Debug, Args)]
pub struct JiraCommentDeleteArgs {
    /// Issue key (e.g. PROJ-123)
    pub key: String,

    /// Comment ID
    pub comment_id: String,
}

#[derive(Debug, Args)]
pub struct JiraRemoteLinkDeleteArgs {
    /// Issue key (e.g. PROJ-123)
    pub key: String,

    /// Remote link ID
    pub link_id: String,
}

#[derive(Debug, Args)]
pub struct JiraNotifyArgs {
    /// Issue key (e.g. PROJ-123)
    pub key: String,

    /// Notification subject
    #[arg(long, short)]
    pub subject: String,

    /// Notification body. Use @file to read from file, or - for stdin
    #[arg(long, short)]
    pub body: String,

    /// Recipient account IDs (repeatable)
    #[arg(long = "to", value_name = "ACCOUNT_ID")]
    pub to: Vec<String>,
}

#[derive(Debug, Args)]
pub struct JiraCreateMetaArgs {
    /// Filter by project key
    #[arg(long)]
    pub project: Option<String>,

    /// Filter by issue type name
    #[arg(long, short = 't')]
    pub issue_type: Option<String>,
}

#[derive(Debug, Args)]
pub struct JiraLabelsArgs {
    /// Max results per page
    #[arg(long, short, default_value = "1000")]
    pub limit: u32,

    /// Fetch all results (auto-paginate)
    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct JiraIdArgs {
    /// Resource ID
    pub id: String,
}

#[derive(Debug, Args)]
pub struct JiraBulkCreateArgs {
    /// JSON input: array of field objects or {"issueUpdates": [...]}.
    /// Use @file to read from file, or - for stdin.
    #[arg(long, short)]
    pub input: String,
}

#[derive(Debug, Args)]
pub struct JiraArchiveArgs {
    /// Issue key(s) to archive (e.g. PROJ-123). Repeat for bulk.
    #[arg(required = true)]
    pub keys: Vec<String>,
}

#[derive(Debug, Args)]
pub struct JiraUnarchiveArgs {
    /// Issue key(s) to unarchive (e.g. PROJ-123). Repeat for bulk.
    #[arg(required = true)]
    pub keys: Vec<String>,
}

// -- Jira issue subtree (`atl jira issue …`) --
//
// The flat issue surface (`view`, `create`, `update`, `delete`, …) above is
// kept untouched; this nested wrapper only hosts new commands like `check`
// that don't have a flat counterpart.

/// Wrapper for `atl jira issue <subcommand>`.
#[derive(Debug, Args)]
pub struct JiraIssueCommand {
    #[command(subcommand)]
    pub command: JiraIssueSubcommand,
}

/// Nested issue-scoped subcommands.
#[derive(Debug, Subcommand)]
pub enum JiraIssueSubcommand {
    /// Verify an issue has values for required/warning fields
    Check(JiraCheckArgs),
}

/// Arguments for `atl jira issue check <KEY>`.
///
/// `--require` and `--warn` are repeatable and also accept comma-separated
/// values. When neither is given, a curated default warn-list is applied.
/// When `--require` is given alone (no `--warn`), the curated list is **not**
/// applied — only the explicitly named fields are checked.
#[derive(Debug, Args)]
pub struct JiraCheckArgs {
    /// Issue key (e.g. PROJ-123)
    pub key: String,

    /// Field that must be set; missing fails the command. Repeatable; comma-lists allowed.
    #[arg(long, value_delimiter = ',')]
    pub require: Vec<String>,

    /// Field reported as a warning when missing (never fails). Repeatable; comma-lists allowed.
    #[arg(long, value_delimiter = ',')]
    pub warn: Vec<String>,
}
