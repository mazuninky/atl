use clap::{Args, Subcommand};

use super::{JiraIdArgs, JiraListGetSubcommand};

// -- Jira Admin: Screens --

#[derive(Debug, Args)]
pub struct JiraScreenCommand {
    #[command(subcommand)]
    pub command: JiraScreenSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraScreenSubcommand {
    /// List screens
    List,
    /// Get a screen by ID
    Get(JiraIdArgs),
    /// Create a screen
    Create(JiraScreenCreateArgs),
    /// Delete a screen
    Delete(JiraIdArgs),
    /// List tabs for a screen
    Tabs(JiraIdArgs),
    /// List fields for a screen tab
    Fields(JiraScreenFieldsArgs),
}

#[derive(Debug, Args)]
pub struct JiraScreenCreateArgs {
    /// Screen name
    #[arg(long, short)]
    pub name: String,
    /// Description
    #[arg(long, short)]
    pub description: Option<String>,
}

#[derive(Debug, Args)]
pub struct JiraScreenFieldsArgs {
    /// Screen ID
    pub screen_id: String,

    /// Tab ID
    pub tab_id: String,
}

// -- Jira Admin: Workflows --

#[derive(Debug, Args)]
pub struct JiraWorkflowCommand {
    #[command(subcommand)]
    pub command: JiraListGetSubcommand,
}

// -- Jira Admin: Schemes --

// -- Field Configuration --

#[derive(Debug, Args)]
pub struct JiraFieldConfigCommand {
    #[command(subcommand)]
    pub command: JiraFieldConfigSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraFieldConfigSubcommand {
    /// List all
    List,
    /// Get by ID
    Get(JiraIdArgs),
    /// Create
    Create(JiraSchemeCreateArgs),
    /// Delete
    Delete(JiraIdArgs),
}

// -- Workflow Scheme --

#[derive(Debug, Args)]
pub struct JiraWorkflowSchemeCommand {
    #[command(subcommand)]
    pub command: JiraCrudSubcommand,
}

// -- Permission Scheme --

#[derive(Debug, Args)]
pub struct JiraPermissionSchemeCommand {
    #[command(subcommand)]
    pub command: JiraCrudSubcommand,
}

// -- Notification Scheme --

#[derive(Debug, Args)]
pub struct JiraNotificationSchemeCommand {
    #[command(subcommand)]
    pub command: JiraCrudSubcommand,
}

// -- Issue Security Scheme --

#[derive(Debug, Args)]
pub struct JiraIssueSecuritySchemeCommand {
    #[command(subcommand)]
    pub command: JiraCrudSubcommand,
}

/// Reusable CRUD subcommand for scheme resources
#[derive(Debug, Subcommand)]
pub enum JiraCrudSubcommand {
    /// List all
    List,
    /// Get by ID
    Get(JiraIdArgs),
    /// Create
    Create(JiraSchemeCreateArgs),
    /// Update
    Update(JiraSchemeUpdateArgs),
    /// Delete
    Delete(JiraIdArgs),
}

#[derive(Debug, Args)]
pub struct JiraSchemeCreateArgs {
    /// Name
    #[arg(long, short)]
    pub name: String,
    /// Description
    #[arg(long, short)]
    pub description: Option<String>,
}

#[derive(Debug, Args)]
pub struct JiraSchemeUpdateArgs {
    /// Resource ID
    pub id: String,
    /// New name
    #[arg(long, short)]
    pub name: Option<String>,
    /// New description
    #[arg(long, short)]
    pub description: Option<String>,
}

// -- Jira Admin: Issue Type Schemes --

#[derive(Debug, Args)]
pub struct JiraIssueTypeSchemeCommand {
    #[command(subcommand)]
    pub command: JiraIssueTypeSchemeSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraIssueTypeSchemeSubcommand {
    /// List issue type schemes
    List,
    /// Get an issue type scheme by ID
    Get(JiraIdArgs),
    /// Create an issue type scheme
    Create(JiraIssueTypeSchemeCreateArgs),
    /// Update an issue type scheme
    Update(JiraSchemeUpdateArgs),
    /// Delete an issue type scheme
    Delete(JiraIdArgs),
}

#[derive(Debug, Args)]
pub struct JiraIssueTypeSchemeCreateArgs {
    /// Scheme name
    #[arg(long, short)]
    pub name: String,
    /// Description
    #[arg(long, short)]
    pub description: Option<String>,
    /// Default issue type ID
    #[arg(long)]
    pub default_issue_type_id: Option<String>,
}
