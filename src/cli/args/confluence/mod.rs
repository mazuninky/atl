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
    /// Confluence storage format (XHTML) — the canonical wire format.
    Storage,
    /// Server-rendered HTML preview (read-only).
    View,
    /// Markdown rendered from storage XHTML (converted client-side).
    #[default]
    Markdown,
    /// Atlassian Document Format — the native Cloud JSON representation.
    Adf,
}

impl BodyFormat {
    /// Stable short identifier suitable for log/diagnostic output.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Storage => "storage",
            Self::View => "view",
            Self::Markdown => "markdown",
            Self::Adf => "adf",
        }
    }

    /// Wire-format value to send to the Confluence v2 `body-format` query
    /// parameter.
    ///
    /// `Markdown` and `Storage` both fetch storage XHTML from the server;
    /// markdown conversion happens client-side. `Adf` requests the native
    /// `atlas_doc_format` representation. `View` is for rendered HTML
    /// preview.
    pub fn wire_format(&self) -> &'static str {
        match self {
            Self::Storage | Self::Markdown => "storage",
            Self::View => "view",
            Self::Adf => "atlas_doc_format",
        }
    }

    /// File extension to use when persisting a body of this format to disk.
    ///
    /// Used by `atl confluence export` so the on-disk filename matches the
    /// content shape: storage XHTML lives in `.xhtml`, the server-rendered
    /// HTML preview in `.html`, ADF (a stringified JSON document) in
    /// `.json`, and markdown in `.md`.
    pub fn file_extension(&self) -> &'static str {
        match self {
            Self::Storage => "xhtml",
            Self::View => "html",
            Self::Markdown => "md",
            Self::Adf => "json",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum InputFormat {
    /// Confluence storage format (XHTML) — sent to the server unchanged.
    Storage,
    /// Markdown — converted to storage format client-side.
    #[default]
    Markdown,
    /// Atlassian Document Format JSON — sent natively as
    /// `atlas_doc_format`.
    Adf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_extension_storage_is_xhtml() {
        // Storage XHTML is canonical Confluence wire format — `.xhtml` makes
        // the on-disk type discoverable by editors that key off extensions.
        assert_eq!(BodyFormat::Storage.file_extension(), "xhtml");
    }

    #[test]
    fn file_extension_view_is_html() {
        // The view format is server-rendered HTML preview; `.html` matches.
        assert_eq!(BodyFormat::View.file_extension(), "html");
    }

    #[test]
    fn file_extension_markdown_is_md() {
        // Markdown export is the new default — must land in `.md`, not the
        // legacy hard-coded `.html`.
        assert_eq!(BodyFormat::Markdown.file_extension(), "md");
    }

    #[test]
    fn file_extension_adf_is_json() {
        // ADF is a stringified JSON document; `.json` reflects that.
        assert_eq!(BodyFormat::Adf.file_extension(), "json");
    }
}
