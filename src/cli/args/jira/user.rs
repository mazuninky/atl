use clap::{Args, Subcommand};

// -- Jira User --

#[derive(Debug, Args)]
pub struct JiraUserCommand {
    #[command(subcommand)]
    pub command: JiraUserSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraUserSubcommand {
    /// Search users
    Search(JiraUserSearchArgs),

    /// Get user by account ID
    Get(JiraUserGetArgs),

    /// List all users
    List(JiraUserListArgs),

    /// Create a user
    Create(JiraUserCreateArgs),

    /// Delete a user
    Delete(JiraUserGetArgs),

    /// List users assignable to an issue
    Assignable(JiraUserAssignableArgs),
}

#[derive(Debug, Args)]
pub struct JiraUserSearchArgs {
    /// Search query
    pub query: String,

    /// Max results
    #[arg(long, short, default_value = "50")]
    pub limit: u32,
}

#[derive(Debug, Args)]
pub struct JiraUserGetArgs {
    /// User account ID
    pub account_id: String,
}

#[derive(Debug, Args)]
pub struct JiraUserListArgs {
    /// Max results per page
    #[arg(long, short, default_value = "50")]
    pub limit: u32,

    /// Fetch all results (auto-paginate)
    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct JiraUserCreateArgs {
    /// Email address
    #[arg(long, short)]
    pub email: String,

    /// Display name
    #[arg(long, short)]
    pub display_name: Option<String>,

    /// Products (comma-separated, e.g. "jira-software")
    #[arg(long)]
    pub products: Option<String>,
}

#[derive(Debug, Args)]
pub struct JiraUserAssignableArgs {
    /// Issue key
    pub issue_key: String,

    /// Max results
    #[arg(long, short, default_value = "50")]
    pub limit: u32,
}

// -- Jira Group --

#[derive(Debug, Args)]
pub struct JiraGroupCommand {
    #[command(subcommand)]
    pub command: JiraGroupSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraGroupSubcommand {
    /// List groups
    List,

    /// Get group details
    Get(JiraGroupNameArgs),

    /// Create a group
    Create(JiraGroupNameArgs),

    /// Delete a group
    Delete(JiraGroupNameArgs),

    /// List group members
    Members(JiraGroupMembersArgs),

    /// Add a user to a group
    AddUser(JiraGroupUserArgs),

    /// Remove a user from a group
    RemoveUser(JiraGroupUserArgs),

    /// Search groups
    Search(JiraGroupSearchArgs),
}

#[derive(Debug, Args)]
pub struct JiraGroupSearchArgs {
    /// Search query
    pub query: String,
    /// Max results
    #[arg(long, short, default_value = "50")]
    pub limit: u32,
}

#[derive(Debug, Args)]
pub struct JiraGroupNameArgs {
    /// Group name
    pub name: String,
}

#[derive(Debug, Args)]
pub struct JiraGroupMembersArgs {
    /// Group name
    pub name: String,

    /// Max results
    #[arg(long, short, default_value = "50")]
    pub limit: u32,
}

#[derive(Debug, Args)]
pub struct JiraGroupUserArgs {
    /// Group name
    pub name: String,

    /// User account ID
    pub account_id: String,
}
