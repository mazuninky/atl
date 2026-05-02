use clap::{ArgGroup, Args, Subcommand, ValueEnum};

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
    /// Manage custom field contexts
    Context(JiraFieldContextCommand),
    /// Manage select-list options for a custom field context
    Options(JiraFieldOptionsCommand),
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

// -- Jira Admin: Field Contexts --

#[derive(Debug, Args)]
pub struct JiraFieldContextCommand {
    /// Custom field ID (e.g. "customfield_10010")
    pub field_id: String,

    #[command(subcommand)]
    pub command: JiraFieldContextSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraFieldContextSubcommand {
    /// List contexts for the field
    List(JiraFieldContextListArgs),
    /// Create a context
    Create(JiraFieldContextCreateArgs),
    /// Update a context
    Update(JiraFieldContextUpdateArgs),
    /// Delete a context
    Delete(JiraFieldContextIdArgs),

    /// List projects mapped to a context
    Projects(JiraFieldContextMappingArgs),
    /// Add projects to a context
    AddProjects(JiraFieldContextProjectsArgs),
    /// Remove projects from a context
    RemoveProjects(JiraFieldContextProjectsArgs),

    /// List issue types mapped to a context
    IssueTypes(JiraFieldContextMappingArgs),
    /// Add issue types to a context
    AddIssueTypes(JiraFieldContextIssueTypesArgs),
    /// Remove issue types from a context
    RemoveIssueTypes(JiraFieldContextIssueTypesArgs),
}

#[derive(Debug, Args)]
pub struct JiraFieldContextListArgs {
    /// Max results per page
    #[arg(long, short, default_value = "50")]
    pub limit: u32,

    /// Fetch all results (auto-paginate)
    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct JiraFieldContextCreateArgs {
    /// Context name
    #[arg(long, short)]
    pub name: String,

    /// Description
    #[arg(long, short)]
    pub description: Option<String>,

    /// Issue type ID(s) the context applies to (repeatable)
    #[arg(long = "issue-type-id", value_name = "ID")]
    pub issue_type_ids: Vec<String>,

    /// Project ID(s) the context applies to (repeatable)
    #[arg(long = "project-id", value_name = "ID")]
    pub project_ids: Vec<String>,
}

#[derive(Debug, Args)]
pub struct JiraFieldContextUpdateArgs {
    /// Context ID
    pub context_id: String,

    /// New name
    #[arg(long, short)]
    pub name: Option<String>,

    /// New description
    #[arg(long, short)]
    pub description: Option<String>,
}

#[derive(Debug, Args)]
pub struct JiraFieldContextIdArgs {
    /// Context ID
    pub context_id: String,
}

#[derive(Debug, Args)]
pub struct JiraFieldContextMappingArgs {
    /// Context ID
    pub context_id: String,

    /// Max results per page
    #[arg(long, short, default_value = "50")]
    pub limit: u32,

    /// Fetch all results (auto-paginate)
    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct JiraFieldContextProjectsArgs {
    /// Context ID
    pub context_id: String,

    /// Project ID(s) (repeatable, at least one)
    #[arg(long = "project-id", value_name = "ID", required = true)]
    pub project_ids: Vec<String>,
}

#[derive(Debug, Args)]
pub struct JiraFieldContextIssueTypesArgs {
    /// Context ID
    pub context_id: String,

    /// Issue type ID(s) (repeatable, at least one)
    #[arg(long = "issue-type-id", value_name = "ID", required = true)]
    pub issue_type_ids: Vec<String>,
}

// -- Jira Admin: Field Options --

#[derive(Debug, Args)]
pub struct JiraFieldOptionsCommand {
    /// Custom field ID (e.g. "customfield_10010")
    pub field_id: String,

    /// Context ID
    pub context_id: String,

    #[command(subcommand)]
    pub command: JiraFieldOptionsSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraFieldOptionsSubcommand {
    /// List options in the context
    List(JiraFieldOptionsListArgs),
    /// Add one or more options
    Add(JiraFieldOptionsAddArgs),
    /// Update a single option
    Update(JiraFieldOptionsUpdateArgs),
    /// Delete a single option
    Delete(JiraFieldOptionsDeleteArgs),
    /// Reorder options within the context
    Reorder(JiraFieldOptionsReorderArgs),
}

#[derive(Debug, Args)]
pub struct JiraFieldOptionsListArgs {
    /// Max results per page
    #[arg(long, short, default_value = "100")]
    pub limit: u32,

    /// Fetch all results (auto-paginate)
    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct JiraFieldOptionsAddArgs {
    /// Option value(s) — repeat to add multiple in one call
    #[arg(long = "value", value_name = "VALUE", required = true)]
    pub values: Vec<String>,

    /// Mark the new options as disabled
    #[arg(long)]
    pub disabled: bool,
}

#[derive(Debug, Args)]
pub struct JiraFieldOptionsUpdateArgs {
    /// Option ID
    pub option_id: String,

    /// New option value
    #[arg(long)]
    pub value: Option<String>,

    /// Whether the option is disabled
    #[arg(long)]
    pub disabled: Option<bool>,
}

#[derive(Debug, Args)]
pub struct JiraFieldOptionsDeleteArgs {
    /// Option ID
    pub option_id: String,
}

/// Position keyword for `options reorder --position`.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum JiraFieldOptionsPosition {
    /// Move to the top of the list
    First,
    /// Move to the bottom of the list
    Last,
}

#[derive(Debug, Args)]
#[command(group = ArgGroup::new("reorder_target").required(true).args(["after", "position"]))]
pub struct JiraFieldOptionsReorderArgs {
    /// Option IDs to move (in the order they should appear)
    #[arg(required = true)]
    pub option_ids: Vec<String>,

    /// Insert the moved options after this option ID
    #[arg(long, value_name = "ID")]
    pub after: Option<String>,

    /// Move the options to a fixed position
    #[arg(long, value_enum)]
    pub position: Option<JiraFieldOptionsPosition>,
}
