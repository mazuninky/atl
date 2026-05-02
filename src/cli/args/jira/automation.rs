use clap::{Args, Subcommand};

// -- Jira automation rules (`atl jira automation …`) --
//
// All commands operate on Jira Cloud automation rules via the
// `https://api.atlassian.com/automation/public/jira/{cloudId}` API.
// The cloud-id is fetched lazily on first use and cached in-process.

/// Wrapper for `atl jira automation <subcommand>`.
#[derive(Debug, Args)]
pub struct JiraAutomationCommand {
    #[command(subcommand)]
    pub command: JiraAutomationSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraAutomationSubcommand {
    /// List automation rules
    List(JiraAutomationListArgs),

    /// Get full definition of a rule by UUID
    Get(JiraAutomationUuidArgs),

    /// Create a new rule from a JSON body (`<literal>`, `@file`, or `-` for stdin)
    Create(JiraAutomationBodyArgs),

    /// Update an existing rule with a JSON body
    Update(JiraAutomationUpdateArgs),

    /// Enable a rule
    Enable(JiraAutomationUuidArgs),

    /// Disable a rule
    Disable(JiraAutomationUuidArgs),

    /// Delete a rule (must be disabled first per Atlassian)
    Delete(JiraAutomationDeleteArgs),
}

#[derive(Debug, Args)]
pub struct JiraAutomationListArgs {
    /// Max results per page
    #[arg(long, short)]
    pub limit: Option<u32>,

    /// Pagination cursor returned by a previous `list` call
    #[arg(long)]
    pub cursor: Option<String>,
}

#[derive(Debug, Args)]
pub struct JiraAutomationUuidArgs {
    /// Rule UUID
    pub uuid: String,
}

#[derive(Debug, Args)]
pub struct JiraAutomationBodyArgs {
    /// JSON rule definition. Literal string, `@path/to/file.json`, or `-` for stdin.
    #[arg(long, short)]
    pub body: String,
}

#[derive(Debug, Args)]
pub struct JiraAutomationUpdateArgs {
    /// Rule UUID
    pub uuid: String,

    /// JSON rule definition. Literal string, `@path/to/file.json`, or `-` for stdin.
    #[arg(long, short)]
    pub body: String,
}

#[derive(Debug, Args)]
pub struct JiraAutomationDeleteArgs {
    /// Rule UUID
    pub uuid: String,

    /// Skip the confirmation prompt
    #[arg(long)]
    pub force: bool,
}
