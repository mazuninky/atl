use camino::Utf8PathBuf;
use clap::{Args, Subcommand};

use super::ConfluenceContentTypePropertyCommand;

#[derive(Debug, Args)]
pub struct ConfluenceAttachmentsArgs {
    /// Page ID
    pub page_id: String,

    /// Max results
    #[arg(long, short, default_value = "25")]
    pub limit: u32,

    /// Glob pattern to filter attachment filenames (e.g. "*.pdf")
    #[arg(long)]
    pub pattern: Option<String>,

    /// Filter by media type (e.g. "image/png", "application/pdf")
    #[arg(long)]
    pub media_type: Option<String>,

    /// Filter by filename
    #[arg(long)]
    pub filename: Option<String>,
}

#[derive(Debug, Args)]
pub struct ConfluenceAttachmentIdArgs {
    /// Attachment ID
    pub attachment_id: String,
}

#[derive(Debug, Args)]
pub struct ConfluenceUploadAttachmentArgs {
    /// Page ID
    pub page_id: String,

    /// Path to the file to upload
    #[arg(long, short)]
    pub file: Utf8PathBuf,
}

#[derive(Debug, Args)]
pub struct ConfluenceAttachmentCommand {
    #[command(subcommand)]
    pub command: ConfluenceAttachmentSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfluenceAttachmentSubcommand {
    /// List attachments for a page
    List(ConfluenceAttachmentsArgs),

    /// Get attachment by ID (v2)
    Get(ConfluenceAttachmentIdArgs),

    /// Upload an attachment to a page
    Upload(ConfluenceUploadAttachmentArgs),

    /// Delete an attachment
    Delete(ConfluenceAttachmentIdArgs),

    /// Download an attachment
    Download(ConfluenceAttachmentDownloadArgs),

    /// List attachment labels (v2)
    Labels(ConfluenceAttachmentIdLimitArgs),

    /// List attachment comments (v2)
    Comments(ConfluenceAttachmentIdLimitArgs),

    /// Attachment operations (v2)
    Operations(ConfluenceAttachmentIdArgs),

    /// Attachment version history (v2)
    Versions(ConfluenceAttachmentIdLimitArgs),

    /// Get specific attachment version details (v2)
    VersionDetails(ConfluenceAttachmentVersionDetailArgs),

    /// Attachment property management (v2)
    Property(ConfluenceContentTypePropertyCommand),
}

#[derive(Debug, Args)]
pub struct ConfluenceAttachmentVersionDetailArgs {
    /// Attachment ID
    pub attachment_id: String,

    /// Version number
    pub version: u32,
}

#[derive(Debug, Args)]
pub struct ConfluenceAttachmentDownloadArgs {
    /// Attachment ID
    pub attachment_id: String,

    /// ID of the page that owns the attachment (required by Confluence REST)
    #[arg(long)]
    pub page_id: String,

    /// Output file path
    #[arg(long, short)]
    pub output: Option<Utf8PathBuf>,
}

#[derive(Debug, Args)]
pub struct ConfluenceAttachmentIdLimitArgs {
    /// Attachment ID
    pub attachment_id: String,

    /// Max results
    #[arg(long, short, default_value = "25")]
    pub limit: u32,
}
