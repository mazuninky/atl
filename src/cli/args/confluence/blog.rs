use clap::{Args, Subcommand};

use super::{BodyFormat, ConfluenceContentTypePropertyCommand, InputFormat};

#[derive(Debug, Args)]
pub struct ConfluenceBlogCommand {
    #[command(subcommand)]
    pub command: ConfluenceBlogSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfluenceBlogSubcommand {
    /// List blog posts
    List(ConfluenceBlogListArgs),

    /// Read a blog post
    Read(ConfluenceBlogReadArgs),

    /// Create a blog post
    Create(ConfluenceBlogCreateArgs),

    /// Update a blog post
    Update(ConfluenceBlogUpdateArgs),

    /// Delete a blog post
    Delete(ConfluenceBlogDeleteArgs),

    /// List blog post attachments (v2)
    Attachments(ConfluenceBlogIdLimitArgs),

    /// List blog post labels (v2)
    Labels(ConfluenceBlogIdArgs),

    /// List blog post footer comments (v2)
    FooterComments(ConfluenceBlogIdLimitArgs),

    /// List blog post inline comments (v2)
    InlineComments(ConfluenceBlogIdLimitArgs),

    /// List blog post versions (v2)
    Versions(ConfluenceBlogIdLimitArgs),

    /// Blog post likes (v2)
    Likes(ConfluenceBlogIdArgs),

    /// Blog post operations (v2)
    Operations(ConfluenceBlogIdArgs),

    /// Get specific blog post version details (v2)
    VersionDetails(ConfluenceBlogVersionDetailArgs),

    /// Get like count for a blog post (v2)
    LikesCount(ConfluenceBlogIdArgs),

    /// Get users who liked a blog post (v2)
    LikesUsers(ConfluenceBlogIdArgs),

    /// List custom content in a blog post (v2)
    CustomContent(ConfluenceBlogCustomContentArgs),

    /// Redact content from a blog post (v2)
    Redact(ConfluenceBlogIdArgs),

    /// Blog post property management (v2)
    Property(ConfluenceContentTypePropertyCommand),
}

#[derive(Debug, Args)]
pub struct ConfluenceBlogDeleteArgs {
    /// Blog post ID
    pub blog_id: String,

    /// Permanently delete (purge) instead of moving to trash
    #[arg(long)]
    pub purge: bool,

    /// Delete draft version only
    #[arg(long)]
    pub draft: bool,
}

#[derive(Debug, Args)]
pub struct ConfluenceBlogListArgs {
    /// Space key to filter by
    #[arg(long, short)]
    pub space: Option<String>,

    /// Max results
    #[arg(long, short, default_value = "25")]
    pub limit: u32,
}

#[derive(Debug, Args)]
pub struct ConfluenceBlogReadArgs {
    /// Blog post ID
    pub blog_id: String,

    /// Body format
    #[arg(long, default_value = "markdown", value_enum)]
    pub body_format: BodyFormat,

    /// Include labels in the response
    #[arg(long)]
    pub include_labels: bool,

    /// Include properties in the response
    #[arg(long)]
    pub include_properties: bool,

    /// Include operations in the response
    #[arg(long)]
    pub include_operations: bool,

    /// Include version details in the response
    #[arg(long)]
    pub include_versions: bool,

    /// Include collaborators in the response
    #[arg(long)]
    pub include_collaborators: bool,
}

#[derive(Debug, Args)]
pub struct ConfluenceBlogCreateArgs {
    /// Space key (resolved to ID internally)
    #[arg(long, short, required_unless_present = "space_id")]
    pub space: Option<String>,

    /// Space ID (numeric, skips key-to-ID lookup)
    #[arg(long, conflicts_with = "space")]
    pub space_id: Option<String>,

    /// Blog post title
    #[arg(long, short)]
    pub title: String,

    /// Blog post body. Use @file to read from file, or - for stdin
    #[arg(long, short)]
    pub body: String,

    /// Input format for the body
    #[arg(long, default_value = "markdown", value_enum)]
    pub input_format: InputFormat,

    /// Create as a private (personal) blog post
    #[arg(long)]
    pub private: bool,
}

#[derive(Debug, Args)]
pub struct ConfluenceBlogUpdateArgs {
    /// Blog post ID
    pub blog_id: String,

    /// New title
    #[arg(long, short)]
    pub title: String,

    /// New body. Use @file to read from file, or - for stdin
    #[arg(long, short)]
    pub body: String,

    /// Version number
    #[arg(long)]
    pub version: u64,

    /// Input format for the body
    #[arg(long, default_value = "markdown", value_enum)]
    pub input_format: InputFormat,

    /// Version comment/message
    #[arg(long)]
    pub version_message: Option<String>,
}

#[derive(Debug, Args)]
pub struct ConfluenceBlogIdArgs {
    /// Blog post ID
    pub blog_id: String,
}

#[derive(Debug, Args)]
pub struct ConfluenceBlogIdLimitArgs {
    /// Blog post ID
    pub blog_id: String,

    /// Max results
    #[arg(long, short, default_value = "25")]
    pub limit: u32,
}

#[derive(Debug, Args)]
pub struct ConfluenceBlogVersionDetailArgs {
    /// Blog post ID
    pub blog_id: String,

    /// Version number
    pub version: u32,
}

#[derive(Debug, Args)]
pub struct ConfluenceBlogCustomContentArgs {
    /// Blog post ID
    pub blog_id: String,

    /// Custom content type
    #[arg(long, short = 't')]
    pub content_type: String,

    /// Max results
    #[arg(long, short, default_value = "25")]
    pub limit: u32,
}
