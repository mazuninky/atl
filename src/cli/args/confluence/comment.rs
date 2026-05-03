use clap::{Args, Subcommand};

use super::{BodyFormat, ConfluenceContentTypePropertyCommand, InputFormat};

#[derive(Debug, Args)]
pub struct ConfluenceCommentsArgs {
    /// Page ID
    pub page_id: String,

    /// Max results
    #[arg(long, short, default_value = "25")]
    pub limit: u32,
}

#[derive(Debug, Args)]
pub struct ConfluenceCreateCommentArgs {
    /// Page ID to comment on
    pub page_id: String,

    /// Comment body (storage format). Use @file to read from file, or - for stdin
    #[arg(long, short)]
    pub body: String,

    /// Parent comment ID (for replies)
    #[arg(long)]
    pub parent: Option<String>,
}

#[derive(Debug, Args)]
pub struct ConfluenceCommentIdArgs {
    /// Comment ID
    pub comment_id: String,
}

// =========================================================================
// Footer Comments (v2)
// =========================================================================

#[derive(Debug, Args)]
pub struct ConfluenceFooterCommentCommand {
    #[command(subcommand)]
    pub command: ConfluenceFooterCommentSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfluenceFooterCommentSubcommand {
    /// List footer comments for a page
    List(ConfluenceFooterCommentListArgs),

    /// Get a footer comment by ID
    Get(ConfluenceFooterCommentGetArgs),

    /// Create a footer comment
    Create(ConfluenceFooterCommentCreateArgs),

    /// Update a footer comment
    Update(ConfluenceFooterCommentUpdateArgs),

    /// Delete a footer comment
    Delete(ConfluenceCommentIdArgs),

    /// List child comments
    Children(ConfluenceCommentIdLimitArgs),

    /// List comment versions
    Versions(ConfluenceCommentIdLimitArgs),

    /// Comment likes
    Likes(ConfluenceCommentIdArgs),

    /// Get permitted operations for a footer comment
    Operations(ConfluenceCommentIdArgs),

    /// Get like count for a footer comment
    LikesCount(ConfluenceCommentIdArgs),

    /// Get users who liked a footer comment
    LikesUsers(ConfluenceCommentIdArgs),

    /// Get specific version details for a footer comment
    VersionDetails(ConfluenceCommentVersionDetailArgs),

    /// Footer comment property management (v2)
    Property(ConfluenceContentTypePropertyCommand),
}

#[derive(Debug, Args)]
pub struct ConfluenceFooterCommentListArgs {
    /// Page ID
    pub page_id: String,

    /// Max results
    #[arg(long, short, default_value = "25")]
    pub limit: u32,

    /// Body format for the comment body field in the response
    #[arg(long, default_value = "markdown", value_enum)]
    pub body_format: BodyFormat,

    /// Strip MyST-style directives (`:::info`/`:::warning`/etc.) from
    /// markdown output. No effect when `--body-format` is not `markdown`.
    #[arg(long)]
    pub no_directives: bool,
}

#[derive(Debug, Args)]
pub struct ConfluenceFooterCommentGetArgs {
    /// Comment ID
    pub comment_id: String,

    /// Body format for the comment body field in the response
    #[arg(long, default_value = "markdown", value_enum)]
    pub body_format: BodyFormat,

    /// Strip MyST-style directives (`:::info`/`:::warning`/etc.) from
    /// markdown output. No effect when `--body-format` is not `markdown`.
    #[arg(long)]
    pub no_directives: bool,
}

#[derive(Debug, Args)]
pub struct ConfluenceFooterCommentCreateArgs {
    /// Page ID
    pub page_id: String,

    /// Comment body. Use @file to read from file, or - for stdin
    #[arg(long, short)]
    pub body: String,

    /// Input format for the body
    #[arg(long, default_value = "markdown", value_enum)]
    pub input_format: InputFormat,
}

#[derive(Debug, Args)]
pub struct ConfluenceFooterCommentUpdateArgs {
    /// Comment ID
    pub comment_id: String,

    /// Comment body. Use @file to read from file, or - for stdin
    #[arg(long, short)]
    pub body: String,

    /// Version number
    #[arg(long)]
    pub version: u32,

    /// Input format for the body
    #[arg(long, default_value = "markdown", value_enum)]
    pub input_format: InputFormat,
}

#[derive(Debug, Args)]
pub struct ConfluenceCommentIdLimitArgs {
    /// Comment ID
    pub comment_id: String,

    /// Max results
    #[arg(long, short, default_value = "25")]
    pub limit: u32,
}

#[derive(Debug, Args)]
pub struct ConfluenceCommentVersionDetailArgs {
    /// Comment ID
    pub comment_id: String,

    /// Version number
    pub version: u32,
}

// =========================================================================
// Inline Comments (v2)
// =========================================================================

#[derive(Debug, Args)]
pub struct ConfluenceInlineCommentCommand {
    #[command(subcommand)]
    pub command: ConfluenceInlineCommentSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfluenceInlineCommentSubcommand {
    /// List inline comments for a page
    List(ConfluenceInlineCommentListArgs),

    /// Get an inline comment by ID
    Get(ConfluenceInlineCommentGetArgs),

    /// Create an inline comment
    Create(ConfluenceInlineCommentCreateArgs),

    /// Update an inline comment
    Update(ConfluenceInlineCommentUpdateArgs),

    /// Delete an inline comment
    Delete(ConfluenceCommentIdArgs),

    /// List child comments
    Children(ConfluenceCommentIdLimitArgs),

    /// List comment versions
    Versions(ConfluenceCommentIdLimitArgs),

    /// Comment likes
    Likes(ConfluenceCommentIdArgs),

    /// Get permitted operations for an inline comment
    Operations(ConfluenceCommentIdArgs),

    /// Get like count for an inline comment
    LikesCount(ConfluenceCommentIdArgs),

    /// Get users who liked an inline comment
    LikesUsers(ConfluenceCommentIdArgs),

    /// Get specific version details for an inline comment
    VersionDetails(ConfluenceCommentVersionDetailArgs),

    /// Inline comment property management (v2)
    Property(ConfluenceContentTypePropertyCommand),
}

#[derive(Debug, Args)]
pub struct ConfluenceInlineCommentGetArgs {
    /// Comment ID
    pub comment_id: String,

    /// Body format for the comment body field in the response
    #[arg(long, default_value = "markdown", value_enum)]
    pub body_format: BodyFormat,

    /// Strip MyST-style directives (`:::info`/`:::warning`/etc.) from
    /// markdown output. No effect when `--body-format` is not `markdown`.
    #[arg(long)]
    pub no_directives: bool,
}

#[derive(Debug, Args)]
pub struct ConfluenceInlineCommentCreateArgs {
    /// Page ID
    pub page_id: String,

    /// Comment body. Use @file to read from file, or - for stdin
    #[arg(long, short)]
    pub body: String,

    /// Inline marker reference (from the page content)
    #[arg(long)]
    pub inline_marker_ref: String,

    /// Text selection to highlight
    #[arg(long)]
    pub text_selection: Option<String>,

    /// Input format for the body
    #[arg(long, default_value = "markdown", value_enum)]
    pub input_format: InputFormat,
}

#[derive(Debug, Args)]
pub struct ConfluenceInlineCommentListArgs {
    /// Page ID
    pub page_id: String,

    /// Max results
    #[arg(long, short, default_value = "25")]
    pub limit: u32,

    /// Filter by resolution status (open, resolved, dangling)
    #[arg(long)]
    pub resolution_status: Option<String>,

    /// Body format for the comment body field in the response
    #[arg(long, default_value = "markdown", value_enum)]
    pub body_format: BodyFormat,

    /// Strip MyST-style directives (`:::info`/`:::warning`/etc.) from
    /// markdown output. No effect when `--body-format` is not `markdown`.
    #[arg(long)]
    pub no_directives: bool,
}

#[derive(Debug, Args)]
pub struct ConfluenceInlineCommentUpdateArgs {
    /// Comment ID
    pub comment_id: String,

    /// Comment body. Use @file to read from file, or - for stdin
    #[arg(long, short)]
    pub body: String,

    /// Version number
    #[arg(long)]
    pub version: u32,

    /// Mark as resolved or unresolved
    #[arg(long)]
    pub resolved: Option<bool>,

    /// Input format for the body
    #[arg(long, default_value = "markdown", value_enum)]
    pub input_format: InputFormat,
}
