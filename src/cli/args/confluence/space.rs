use clap::{Args, Subcommand};

use super::ConfluenceContentTypePropertyCommand;
use super::ConfluenceLimitArgs;

#[derive(Debug, Args)]
pub struct ConfluenceSpaceCommand {
    #[command(subcommand)]
    pub command: ConfluenceSpaceSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfluenceSpaceSubcommand {
    /// List spaces
    List(ConfluenceLimitArgs),

    /// Get space by ID
    Get(ConfluenceSpaceIdArgs),

    /// Create a space
    Create(ConfluenceSpaceCreateArgs),

    /// Delete a space
    Delete(ConfluenceSpaceIdArgs),

    /// List pages in space
    Pages(ConfluenceSpaceIdLimitArgs),

    /// List blog posts in space
    Blogposts(ConfluenceSpaceIdLimitArgs),

    /// List labels in space
    Labels(ConfluenceSpaceIdLimitArgs),

    /// List space permissions
    Permissions(ConfluenceSpaceIdLimitArgs),

    /// Available space permissions
    PermissionsAvailable,

    /// List labels of content in space
    ContentLabels(ConfluenceSpaceIdLimitArgs),

    /// List custom content in space
    CustomContent(ConfluenceSpaceCustomContentArgs),

    /// List permitted operations for space
    Operations(ConfluenceSpaceIdArgs),

    /// Get space role assignments
    RoleAssignments(ConfluenceSpaceIdLimitArgs),

    /// Set space role assignments
    SetRoleAssignments(ConfluenceSpaceSetRoleAssignmentsArgs),

    /// Space property management (v2)
    Property(ConfluenceContentTypePropertyCommand),

    /// Space role management
    Role(ConfluenceSpaceRoleCommand),
}

#[derive(Debug, Args)]
pub struct ConfluenceSpaceIdArgs {
    /// Space ID
    pub space_id: String,
}

#[derive(Debug, Args)]
pub struct ConfluenceSpaceIdLimitArgs {
    /// Space ID
    pub space_id: String,

    /// Max results
    #[arg(long, short, default_value = "25")]
    pub limit: u32,
}

#[derive(Debug, Args)]
pub struct ConfluenceSpaceCreateArgs {
    /// Space key
    #[arg(long, short)]
    pub key: String,

    /// Space name
    #[arg(long, short)]
    pub name: String,

    /// Space description
    #[arg(long, short)]
    pub description: Option<String>,

    /// Create as a private (personal) space
    #[arg(long)]
    pub private: bool,

    /// Space alias
    #[arg(long)]
    pub alias: Option<String>,

    /// Template key to use for the space homepage
    #[arg(long)]
    pub template_key: Option<String>,
}

#[derive(Debug, Args)]
pub struct ConfluenceSpaceCustomContentArgs {
    /// Space ID
    pub space_id: String,

    /// Custom content type
    #[arg(long, short = 't')]
    pub content_type: String,

    /// Max results
    #[arg(long, short, default_value = "25")]
    pub limit: u32,
}

#[derive(Debug, Args)]
pub struct ConfluenceSpaceSetRoleAssignmentsArgs {
    /// Space ID
    pub space_id: String,

    /// Role assignments (JSON). Use @file to read from file, or - for stdin
    #[arg(long, short)]
    pub body: String,
}

// =========================================================================
// Space Roles (v2)
// =========================================================================

#[derive(Debug, Args)]
pub struct ConfluenceSpaceRoleCommand {
    #[command(subcommand)]
    pub command: ConfluenceSpaceRoleSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfluenceSpaceRoleSubcommand {
    /// List roles for a space
    List(ConfluenceSpaceIdLimitArgs),

    /// Get a role by ID
    Get(ConfluenceSpaceRoleIdArgs),

    /// Create a role
    Create(ConfluenceSpaceRoleCreateArgs),

    /// Update a role
    Update(ConfluenceSpaceRoleUpdateArgs),

    /// Delete a role
    Delete(ConfluenceSpaceRoleIdArgs),

    /// Get space roles mode
    Mode(ConfluenceSpaceIdArgs),
}

#[derive(Debug, Args)]
pub struct ConfluenceSpaceRoleIdArgs {
    /// Space ID
    pub space_id: String,

    /// Role ID
    pub role_id: String,
}

#[derive(Debug, Args)]
pub struct ConfluenceSpaceRoleCreateArgs {
    /// Space ID
    pub space_id: String,

    /// Role name
    #[arg(long, short)]
    pub name: String,
}

#[derive(Debug, Args)]
pub struct ConfluenceSpaceRoleUpdateArgs {
    /// Space ID
    pub space_id: String,

    /// Role ID
    pub role_id: String,

    /// New role name
    #[arg(long, short)]
    pub name: String,
}
