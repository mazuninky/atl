use clap::{Args, Subcommand};

use super::{ConfluenceContentTypeIdArgs, ConfluenceSpaceIdArgs};

// =========================================================================
// Tasks (v2)
// =========================================================================

#[derive(Debug, Args)]
pub struct ConfluenceTaskCommand {
    #[command(subcommand)]
    pub command: ConfluenceTaskSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfluenceTaskSubcommand {
    /// List tasks
    List(ConfluenceTaskListArgs),

    /// Get a task by ID
    Get(ConfluenceTaskIdArgs),

    /// Update a task (e.g. mark complete/incomplete)
    Update(ConfluenceTaskUpdateArgs),
}

#[derive(Debug, Args)]
pub struct ConfluenceTaskListArgs {
    /// Filter by space ID
    #[arg(long)]
    pub space_id: Option<String>,

    /// Filter by page ID
    #[arg(long)]
    pub page_id: Option<String>,

    /// Filter by status (complete, incomplete)
    #[arg(long)]
    pub status: Option<String>,

    /// Filter by assignee account ID
    #[arg(long)]
    pub assignee: Option<String>,

    /// Max results
    #[arg(long, short, default_value = "25")]
    pub limit: u32,
}

#[derive(Debug, Args)]
pub struct ConfluenceTaskIdArgs {
    /// Task ID
    pub task_id: String,
}

#[derive(Debug, Args)]
pub struct ConfluenceTaskUpdateArgs {
    /// Task ID
    pub task_id: String,

    /// New status (complete, incomplete)
    #[arg(long)]
    pub status: String,
}

// =========================================================================
// Admin Key (v2)
// =========================================================================

#[derive(Debug, Args)]
pub struct ConfluenceAdminKeyCommand {
    #[command(subcommand)]
    pub command: ConfluenceAdminKeySubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfluenceAdminKeySubcommand {
    /// Get admin key status
    Get,

    /// Enable admin key
    Enable,

    /// Disable admin key
    Disable,
}

// =========================================================================
// Classification (v2)
// =========================================================================

#[derive(Debug, Args)]
pub struct ConfluenceClassificationCommand {
    #[command(subcommand)]
    pub command: ConfluenceClassificationSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfluenceClassificationSubcommand {
    /// List classification levels
    List,

    /// Get classification for a page
    GetPage(ConfluenceContentTypeIdArgs),

    /// Set classification for a page
    SetPage(ConfluenceClassificationSetArgs),

    /// Reset classification for a page
    ResetPage(ConfluenceContentTypeIdArgs),

    /// Get classification for a blog post
    GetBlogpost(ConfluenceContentTypeIdArgs),

    /// Set classification for a blog post
    SetBlogpost(ConfluenceClassificationSetArgs),

    /// Reset classification for a blog post
    ResetBlogpost(ConfluenceContentTypeIdArgs),

    /// Get classification for a space
    GetSpace(ConfluenceSpaceIdArgs),

    /// Set classification for a space
    SetSpace(ConfluenceClassificationSetArgs),

    /// Reset classification for a space
    ResetSpace(ConfluenceSpaceIdArgs),

    /// Get classification for a database
    GetDatabase(ConfluenceContentTypeIdArgs),

    /// Set classification for a database
    SetDatabase(ConfluenceClassificationSetArgs),

    /// Reset classification for a database
    ResetDatabase(ConfluenceContentTypeIdArgs),

    /// Get classification for a whiteboard
    GetWhiteboard(ConfluenceContentTypeIdArgs),

    /// Set classification for a whiteboard
    SetWhiteboard(ConfluenceClassificationSetArgs),

    /// Reset classification for a whiteboard
    ResetWhiteboard(ConfluenceContentTypeIdArgs),
}

#[derive(Debug, Args)]
pub struct ConfluenceClassificationSetArgs {
    /// Target resource ID
    pub id: String,

    /// Classification level ID
    #[arg(long)]
    pub classification_id: String,
}

// =========================================================================
// User (v2)
// =========================================================================

#[derive(Debug, Args)]
pub struct ConfluenceUserCommand {
    #[command(subcommand)]
    pub command: ConfluenceUserSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfluenceUserSubcommand {
    /// Bulk lookup users by account IDs
    Bulk(ConfluenceUserBulkArgs),

    /// Check user access
    CheckAccess(ConfluenceUserCheckAccessArgs),

    /// Invite users
    Invite(ConfluenceUserInviteArgs),
}

#[derive(Debug, Args)]
pub struct ConfluenceUserBulkArgs {
    /// Account IDs to look up
    #[arg(required = true)]
    pub account_ids: Vec<String>,
}

#[derive(Debug, Args)]
pub struct ConfluenceUserAccountIdArgs {
    /// User account ID
    pub account_id: String,
}

#[derive(Debug, Args)]
pub struct ConfluenceUserCheckAccessArgs {
    /// User email address
    pub email: String,
}

#[derive(Debug, Args)]
pub struct ConfluenceUserInviteArgs {
    /// Email addresses to invite
    #[arg(required = true)]
    pub emails: Vec<String>,
}

// =========================================================================
// Convert IDs (v2)
// =========================================================================

#[derive(Debug, Args)]
pub struct ConfluenceConvertIdsArgs {
    /// Content IDs to convert
    #[arg(required = true)]
    pub ids: Vec<String>,
}

// =========================================================================
// App Properties (v2)
// =========================================================================

#[derive(Debug, Args)]
pub struct ConfluenceAppPropertyCommand {
    #[command(subcommand)]
    pub command: ConfluenceAppPropertySubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfluenceAppPropertySubcommand {
    /// List app properties (operates on the calling app's properties)
    List,

    /// Get an app property
    Get(ConfluenceAppPropertyKeyArgs),

    /// Set an app property
    Set(ConfluenceAppPropertySetArgs),

    /// Delete an app property
    Delete(ConfluenceAppPropertyKeyArgs),
}

#[derive(Debug, Args)]
pub struct ConfluenceAppPropertyKeyArgs {
    /// Property key
    pub key: String,
}

#[derive(Debug, Args)]
pub struct ConfluenceAppPropertySetArgs {
    /// Property key
    pub key: String,

    /// Property value (JSON). Use @file to read from file, or - for stdin
    #[arg(long)]
    pub value: String,
}
