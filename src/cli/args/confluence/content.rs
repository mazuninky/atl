use clap::{Args, Subcommand};

use super::ConfluenceContentTypePropertyCommand;

#[derive(Debug, Args)]
pub struct ConfluenceContentTypeCommand {
    #[command(subcommand)]
    pub command: ConfluenceContentTypeSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfluenceContentTypeSubcommand {
    /// Create
    Create(ConfluenceContentTypeCreateArgs),

    /// Get by ID
    Get(ConfluenceContentTypeIdArgs),

    /// Delete
    Delete(ConfluenceContentTypeIdArgs),

    /// List ancestors
    Ancestors(ConfluenceContentTypeIdArgs),

    /// List descendants
    Descendants(ConfluenceContentTypeIdLimitArgs),

    /// List direct children
    Children(ConfluenceContentTypeIdLimitArgs),

    /// List operations
    Operations(ConfluenceContentTypeIdArgs),

    /// Content property operations
    Property(ConfluenceContentTypePropertyCommand),
}

#[derive(Debug, Args)]
pub struct ConfluenceContentTypeIdArgs {
    /// Resource ID
    pub id: String,
}

#[derive(Debug, Args)]
pub struct ConfluenceContentTypeIdLimitArgs {
    /// Resource ID
    pub id: String,

    /// Max results
    #[arg(long, short, default_value = "25")]
    pub limit: u32,
}

#[derive(Debug, Args)]
pub struct ConfluenceContentTypeCreateArgs {
    /// Space ID
    #[arg(long, short)]
    pub space_id: String,

    /// Title
    #[arg(long, short)]
    pub title: Option<String>,

    /// Template key (for whiteboards)
    #[arg(long)]
    pub template_key: Option<String>,

    /// Parent content ID
    #[arg(long)]
    pub parent_id: Option<String>,
}

// =========================================================================
// Custom Content (v2)
// =========================================================================

#[derive(Debug, Args)]
pub struct ConfluenceCustomContentCommand {
    #[command(subcommand)]
    pub command: ConfluenceCustomContentSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfluenceCustomContentSubcommand {
    /// List custom content
    List(ConfluenceCustomContentListArgs),

    /// Get custom content by ID
    Get(ConfluenceContentTypeIdArgs),

    /// Create custom content
    Create(ConfluenceCustomContentCreateArgs),

    /// Update custom content
    Update(ConfluenceCustomContentUpdateArgs),

    /// Delete custom content
    Delete(ConfluenceContentTypeIdArgs),

    /// List attachments
    Attachments(ConfluenceContentTypeIdLimitArgs),

    /// List children
    Children(ConfluenceContentTypeIdLimitArgs),

    /// List labels
    Labels(ConfluenceContentTypeIdLimitArgs),

    /// List comments
    Comments(ConfluenceContentTypeIdLimitArgs),

    /// List operations
    Operations(ConfluenceContentTypeIdArgs),

    /// List versions
    Versions(ConfluenceContentTypeIdLimitArgs),

    /// Get specific version details
    VersionDetails(ConfluenceCustomContentVersionDetailArgs),

    /// Custom content property management (v2)
    Property(ConfluenceContentTypePropertyCommand),
}

#[derive(Debug, Args)]
pub struct ConfluenceCustomContentVersionDetailArgs {
    /// Custom content ID
    pub id: String,

    /// Version number
    pub version: u32,
}

#[derive(Debug, Args)]
pub struct ConfluenceCustomContentListArgs {
    /// Custom content type
    #[arg(long, short = 't')]
    pub content_type: Option<String>,

    /// Space ID
    #[arg(long, short)]
    pub space_id: Option<String>,

    /// Max results
    #[arg(long, short, default_value = "25")]
    pub limit: u32,
}

#[derive(Debug, Args)]
pub struct ConfluenceCustomContentCreateArgs {
    /// Custom content type
    #[arg(long, short = 't')]
    pub content_type: String,

    /// Space ID
    #[arg(long, short)]
    pub space_id: String,

    /// Title
    #[arg(long)]
    pub title: String,

    /// Body. Use @file to read from file, or - for stdin
    #[arg(long, short)]
    pub body: String,
}

#[derive(Debug, Args)]
pub struct ConfluenceCustomContentUpdateArgs {
    /// Custom content ID
    pub id: String,

    /// Title
    #[arg(long)]
    pub title: Option<String>,

    /// Body. Use @file to read from file, or - for stdin
    #[arg(long, short)]
    pub body: Option<String>,

    /// Version number
    #[arg(long)]
    pub version: u32,
}
