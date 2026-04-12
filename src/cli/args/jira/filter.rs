use clap::{Args, Subcommand};

use super::JiraIssueKeyArgs;

// -- Jira Worklog --

#[derive(Debug, Args)]
pub struct JiraWorklogCommand {
    #[command(subcommand)]
    pub command: JiraWorklogSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraWorklogSubcommand {
    /// List worklogs for an issue
    List(JiraIssueKeyArgs),

    /// Add a worklog entry
    Add(JiraWorklogAddArgs),

    /// Delete a worklog entry
    Delete(JiraWorklogDeleteArgs),
}

#[derive(Debug, Args)]
pub struct JiraWorklogAddArgs {
    /// Issue key (e.g. PROJ-123)
    pub key: String,

    /// Time spent (e.g. "2h 30m", "1d", "45m")
    #[arg(long, short)]
    pub time_spent: String,

    /// Optional comment
    #[arg(long, short)]
    pub comment: Option<String>,

    /// Start date/time (ISO 8601, e.g. "2024-01-15T09:00:00.000+0000")
    #[arg(long)]
    pub started: Option<String>,
}

#[derive(Debug, Args)]
pub struct JiraWorklogDeleteArgs {
    /// Issue key
    pub key: String,

    /// Worklog ID
    pub worklog_id: String,
}

// -- Jira Filter --

#[derive(Debug, Args)]
pub struct JiraFilterCommand {
    #[command(subcommand)]
    pub command: JiraFilterSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraFilterSubcommand {
    /// List filters
    List(JiraFilterListArgs),

    /// Get a filter by ID
    Get(JiraFilterGetArgs),

    /// Create a new filter
    Create(JiraFilterCreateArgs),

    /// Update a filter
    Update(JiraFilterUpdateArgs),

    /// Delete a filter
    Delete(JiraFilterDeleteArgs),
}

#[derive(Debug, Args)]
pub struct JiraFilterListArgs {
    /// Filter by name
    #[arg(long, short)]
    pub name: Option<String>,
    /// Show only favourite filters
    #[arg(long)]
    pub favourites: bool,
    /// Show only my filters
    #[arg(long)]
    pub mine: bool,
}

#[derive(Debug, Args)]
pub struct JiraFilterGetArgs {
    /// Filter ID
    pub id: String,
}

#[derive(Debug, Args)]
pub struct JiraFilterCreateArgs {
    /// Filter name
    #[arg(long, short)]
    pub name: String,

    /// JQL query
    #[arg(long, short)]
    pub jql: String,

    /// Description
    #[arg(long, short)]
    pub description: Option<String>,

    /// Mark as favourite
    #[arg(long)]
    pub favourite: bool,
}

#[derive(Debug, Args)]
pub struct JiraFilterDeleteArgs {
    /// Filter ID
    pub id: String,
}

#[derive(Debug, Args)]
pub struct JiraFilterUpdateArgs {
    /// Filter ID
    pub id: String,

    /// New filter name
    #[arg(long, short)]
    pub name: Option<String>,

    /// New JQL query
    #[arg(long, short)]
    pub jql: Option<String>,

    /// New description
    #[arg(long, short)]
    pub description: Option<String>,

    /// Mark as favourite
    #[arg(long)]
    pub favourite: Option<bool>,
}
