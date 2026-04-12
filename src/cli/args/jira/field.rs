use clap::{Args, Subcommand};

use super::JiraIdArgs;

// -- Jira Admin: Issue Types --

#[derive(Debug, Args)]
pub struct JiraIssueTypeCommand {
    #[command(subcommand)]
    pub command: JiraIssueTypeSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraIssueTypeSubcommand {
    /// List all issue types
    List,

    /// Get an issue type by ID
    Get(JiraIdArgs),

    /// Create an issue type
    Create(JiraIssueTypeCreateArgs),

    /// Update an issue type
    Update(JiraIssueTypeUpdateArgs),

    /// Delete an issue type
    Delete(JiraIdArgs),
}

#[derive(Debug, Args)]
pub struct JiraIssueTypeCreateArgs {
    /// Issue type name
    #[arg(long, short)]
    pub name: String,

    /// Description
    #[arg(long, short)]
    pub description: Option<String>,

    /// Type (standard or subtask)
    #[arg(long, short = 't', default_value = "standard")]
    pub r#type: String,
}

#[derive(Debug, Args)]
pub struct JiraIssueTypeUpdateArgs {
    /// Issue type ID
    pub id: String,

    /// New name
    #[arg(long, short)]
    pub name: Option<String>,

    /// New description
    #[arg(long, short)]
    pub description: Option<String>,
}

// -- Jira Admin: Priority --

#[derive(Debug, Args)]
pub struct JiraPriorityCommand {
    #[command(subcommand)]
    pub command: JiraPrioritySubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraPrioritySubcommand {
    /// List all priorities
    List,
    /// Get a priority by ID
    Get(JiraIdArgs),
    /// Create a priority
    Create(JiraPriorityCreateArgs),
    /// Update a priority
    Update(JiraPriorityUpdateArgs),
    /// Delete a priority
    Delete(JiraIdArgs),
}

#[derive(Debug, Args)]
pub struct JiraPriorityCreateArgs {
    /// Priority name
    #[arg(long, short)]
    pub name: String,
    /// Description
    #[arg(long, short)]
    pub description: Option<String>,
    /// Status color (hex, e.g. "#ff0000")
    #[arg(long, default_value = "#ffffff")]
    pub status_color: String,
}

#[derive(Debug, Args)]
pub struct JiraPriorityUpdateArgs {
    /// Priority ID
    pub id: String,
    /// New name
    #[arg(long, short)]
    pub name: Option<String>,
    /// New description
    #[arg(long, short)]
    pub description: Option<String>,
    /// New status color
    #[arg(long)]
    pub status_color: Option<String>,
}

// -- Jira Admin: Resolution --

#[derive(Debug, Args)]
pub struct JiraResolutionCommand {
    #[command(subcommand)]
    pub command: JiraResolutionSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraResolutionSubcommand {
    /// List all resolutions
    List,
    /// Get a resolution by ID
    Get(JiraIdArgs),
    /// Create a resolution
    Create(JiraResolutionCreateArgs),
    /// Update a resolution
    Update(JiraResolutionUpdateArgs),
    /// Delete a resolution
    Delete(JiraIdArgs),
}

#[derive(Debug, Args)]
pub struct JiraResolutionCreateArgs {
    /// Resolution name
    #[arg(long, short)]
    pub name: String,
    /// Description
    #[arg(long, short)]
    pub description: Option<String>,
}

#[derive(Debug, Args)]
pub struct JiraResolutionUpdateArgs {
    /// Resolution ID
    pub id: String,
    /// New name
    #[arg(long, short)]
    pub name: Option<String>,
    /// New description
    #[arg(long, short)]
    pub description: Option<String>,
}

// -- Jira Admin: Status --

#[derive(Debug, Args)]
pub struct JiraStatusCommand {
    #[command(subcommand)]
    pub command: JiraStatusSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraStatusSubcommand {
    /// List all statuses
    List,
    /// Get a status by ID
    Get(JiraIdArgs),
    /// List status categories
    Categories,
}

/// Reusable list/get subcommand for read-only admin resources
#[derive(Debug, Subcommand)]
pub enum JiraListGetSubcommand {
    /// List all
    List,

    /// Get by ID
    Get(JiraIdArgs),
}

// -- Jira Admin: Fields --

#[derive(Debug, Args)]
pub struct JiraFieldCommand {
    #[command(subcommand)]
    pub command: JiraFieldSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraFieldSubcommand {
    /// List all fields
    List(JiraFieldListArgs),
    /// Create a custom field
    Create(JiraFieldCreateArgs),
    /// Delete a custom field
    Delete(JiraIdArgs),
    /// Move a custom field to trash
    Trash(JiraIdArgs),
    /// Restore a custom field from trash
    Restore(JiraIdArgs),
}

#[derive(Debug, Args)]
pub struct JiraFieldListArgs {
    /// Show only custom fields
    #[arg(long)]
    pub custom: bool,
}

#[derive(Debug, Args)]
pub struct JiraFieldCreateArgs {
    /// Field name
    #[arg(long, short)]
    pub name: String,
    /// Field type (e.g. "com.atlassian.jira.plugin.system.customfieldtypes:textfield")
    #[arg(long, short = 't')]
    pub r#type: String,
    /// Description
    #[arg(long, short)]
    pub description: Option<String>,
    /// Search key
    #[arg(long)]
    pub search_key: Option<String>,
}
