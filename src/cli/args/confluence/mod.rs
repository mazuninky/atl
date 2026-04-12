use clap::{Args, Subcommand, ValueEnum};

mod admin;
mod attachment;
mod blog;
mod comment;
mod content;
mod label;
mod page;
mod property;
mod space;

pub use admin::*;
pub use attachment::*;
pub use blog::*;
pub use comment::*;
pub use content::*;
pub use label::*;
pub use page::*;
pub use property::*;
pub use space::*;

// -- Confluence --

#[derive(Debug, Args)]
pub struct ConfluenceCommand {
    #[command(subcommand)]
    pub command: ConfluenceSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfluenceSubcommand {
    /// Read a page by ID
    Read(ConfluenceReadArgs),

    /// Get page metadata
    Info(ConfluencePageIdArgs),

    /// Search pages with CQL
    Search(ConfluenceSearchArgs),

    /// List spaces
    #[command(alias = "spaces")]
    Space(ConfluenceSpaceCommand),

    /// List child pages
    Children(ConfluenceChildrenArgs),

    /// Create a new page
    Create(ConfluenceCreateArgs),

    /// Update an existing page
    Update(ConfluenceUpdateArgs),

    /// Delete a page
    Delete(ConfluenceDeleteArgs),

    /// Attachment operations
    #[command(alias = "attachments")]
    Attachment(ConfluenceAttachmentCommand),

    /// List comments for a page (v1, use footer-comment/inline-comment for v2)
    #[command(hide = true)]
    Comments(ConfluenceCommentsArgs),

    /// Find pages by title
    Find(ConfluenceFindArgs),

    /// Add a comment to a page (v1, use footer-comment create for v2)
    #[command(hide = true, name = "create-comment")]
    CreateComment(ConfluenceCreateCommentArgs),

    /// Delete a comment (v1, use footer-comment delete for v2)
    #[command(hide = true, name = "delete-comment")]
    DeleteComment(ConfluenceCommentIdArgs),

    /// Delete an attachment (v1, use attachment delete for v2)
    #[command(hide = true, name = "delete-attachment")]
    DeleteAttachment(ConfluenceAttachmentIdArgs),

    /// Upload an attachment to a page (v1, use attachment upload for v2)
    #[command(hide = true, name = "upload-attachment")]
    UploadAttachment(ConfluenceUploadAttachmentArgs),

    /// Export a page with attachments to a local directory
    Export(ConfluenceExportArgs),

    /// Copy a page tree to another space/parent
    CopyTree(ConfluenceCopyTreeArgs),

    /// Content property operations
    Property(ConfluencePropertyCommand),

    /// Label management
    Label(ConfluenceLabelCommand),

    /// Blog post management
    Blog(ConfluenceBlogCommand),

    /// Page version history
    Versions(ConfluencePageIdLimitArgs),

    /// Get a specific page version
    VersionDetail(ConfluenceVersionDetailArgs),

    /// Page likes
    Likes(ConfluencePageIdArgs),

    /// Page operations/permissions
    Operations(ConfluencePageIdArgs),

    /// Page ancestors
    Ancestors(ConfluencePageIdArgs),

    /// Page descendants
    Descendants(ConfluencePageIdLimitArgs),

    /// Footer comment management (v2)
    FooterComment(ConfluenceFooterCommentCommand),

    /// Inline comment management (v2)
    InlineComment(ConfluenceInlineCommentCommand),

    /// Whiteboard management
    Whiteboard(ConfluenceContentTypeCommand),

    /// Database management
    Database(ConfluenceContentTypeCommand),

    /// Folder management
    Folder(ConfluenceContentTypeCommand),

    /// Custom content management
    CustomContent(ConfluenceCustomContentCommand),

    /// Task management
    Task(ConfluenceTaskCommand),

    /// Admin key management
    AdminKey(ConfluenceAdminKeyCommand),

    /// Content classification management
    Classification(ConfluenceClassificationCommand),

    /// Confluence user operations
    User(ConfluenceUserCommand),

    /// Convert content IDs between formats
    ConvertIds(ConfluenceConvertIdsArgs),

    /// App property management
    AppProperty(ConfluenceAppPropertyCommand),

    /// List pages (v2)
    #[command(name = "page-list")]
    PageList(ConfluencePageListArgs),

    /// Update page title only
    UpdateTitle(ConfluenceUpdateTitleArgs),

    /// Get like count for a page
    LikesCount(ConfluencePageIdArgs),

    /// Get users who liked a page
    LikesUsers(ConfluencePageIdArgs),

    /// List custom content in a page
    PageCustomContent(ConfluencePageCustomContentArgs),

    /// Redact content from a page
    Redact(ConfluenceRedactArgs),
}

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum BodyFormat {
    #[default]
    Storage,
    View,
}

impl BodyFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Storage => "storage",
            Self::View => "view",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum InputFormat {
    /// Confluence storage format (XHTML)
    #[default]
    Storage,
    /// Markdown (converted to storage format)
    Markdown,
}
