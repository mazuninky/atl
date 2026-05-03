use camino::Utf8PathBuf;
use clap::Args;

use super::{BodyFormat, InputFormat};

#[derive(Debug, Args)]
pub struct ConfluenceReadArgs {
    /// Page ID
    pub page_id: String,

    /// Body format
    #[arg(long, default_value = "markdown", value_enum)]
    pub body_format: BodyFormat,

    /// Strip MyST-style directives (`:::info`/`:::warning`/etc.) from
    /// markdown output. No effect when `--body-format` is not `markdown`.
    #[arg(long)]
    pub no_directives: bool,

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

    /// Include favorited-by info in the response
    #[arg(long)]
    pub include_favorited_by: bool,

    /// Open the page in a browser instead of printing
    #[arg(long)]
    pub web: bool,
}

#[derive(Debug, Args)]
pub struct ConfluencePageIdArgs {
    /// Page ID
    pub page_id: String,
}

#[derive(Debug, Args)]
pub struct ConfluencePageIdLimitArgs {
    /// Page ID
    pub page_id: String,

    /// Max results
    #[arg(long, short, default_value = "25")]
    pub limit: u32,
}

#[derive(Debug, Args)]
pub struct ConfluenceSearchArgs {
    /// CQL query
    pub cql: String,

    /// Max results per page
    #[arg(long, short, default_value = "25")]
    pub limit: u32,

    /// Fetch all results (auto-paginate)
    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct ConfluenceLimitArgs {
    /// Max results per page
    #[arg(long, short, default_value = "25")]
    pub limit: u32,

    /// Fetch all results (auto-paginate)
    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct ConfluenceChildrenArgs {
    /// Page ID
    pub page_id: String,

    /// Max results
    #[arg(long, short, default_value = "25")]
    pub limit: u32,

    /// Recursion depth (1 = direct children only)
    #[arg(long, short, default_value = "1")]
    pub depth: u32,

    /// Display as an indented tree
    #[arg(long)]
    pub tree: bool,
}

#[derive(Debug, Args)]
pub struct ConfluenceDeleteArgs {
    /// Page ID
    pub page_id: String,

    /// Permanently delete (purge) instead of moving to trash
    #[arg(long)]
    pub purge: bool,

    /// Delete draft version only
    #[arg(long)]
    pub draft: bool,
}

#[derive(Debug, Args)]
pub struct ConfluenceCreateArgs {
    /// Space key (resolved to ID internally)
    #[arg(long, short, required_unless_present = "space_id")]
    pub space: Option<String>,

    /// Space ID (numeric, skips key-to-ID lookup)
    #[arg(long, conflicts_with = "space")]
    pub space_id: Option<String>,

    /// Page title
    #[arg(long, short)]
    pub title: String,

    /// Page body. Use @file to read from file, or - for stdin
    #[arg(long, short)]
    pub body: String,

    /// Parent page ID
    #[arg(long, conflicts_with = "root_level")]
    pub parent: Option<String>,

    /// Input format for the body
    #[arg(long, default_value = "markdown", value_enum)]
    pub input_format: InputFormat,

    /// Create as a private (personal) page
    #[arg(long)]
    pub private: bool,

    /// Page subtype (e.g. "page", "blog")
    #[arg(long)]
    pub subtype: Option<String>,

    /// Create as embedded content
    #[arg(long)]
    pub embedded: bool,

    /// Create at root level (no parent)
    #[arg(long, conflicts_with = "parent")]
    pub root_level: bool,
}

#[derive(Debug, Args)]
pub struct ConfluenceUpdateArgs {
    /// Page ID
    pub page_id: String,

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
pub struct ConfluenceFindArgs {
    /// Page title to search for
    #[arg(long, short)]
    pub title: String,

    /// Space key to search within
    #[arg(long, short)]
    pub space: Option<String>,

    /// Max results
    #[arg(long, short, default_value = "25")]
    pub limit: u32,
}

#[derive(Debug, Args)]
pub struct ConfluenceExportArgs {
    /// Page ID
    pub page_id: String,

    /// Output directory (default: current directory)
    #[arg(long, short, default_value = ".")]
    pub output_dir: Utf8PathBuf,

    /// Body format for the page content
    #[arg(long, default_value = "markdown", value_enum)]
    pub body_format: BodyFormat,

    /// Strip MyST-style directives (`:::info`/`:::warning`/etc.) from
    /// markdown output. No effect when `--body-format` is not `markdown`.
    #[arg(long)]
    pub no_directives: bool,
}

#[derive(Debug, Args)]
pub struct ConfluenceCopyTreeArgs {
    /// Source page ID (root of the tree to copy)
    pub source_page_id: String,

    /// Target space key (resolved to ID internally)
    #[arg(long, required_unless_present = "target_space_id")]
    pub target_space: Option<String>,

    /// Target space ID (numeric, skips key-to-ID lookup)
    #[arg(long, conflicts_with = "target_space")]
    pub target_space_id: Option<String>,

    /// Target parent page ID
    #[arg(long)]
    pub target_parent: Option<String>,

    /// Maximum depth to copy (0 = source page only)
    #[arg(long, short, default_value = "999")]
    pub depth: u32,

    /// Show what would be copied without making changes
    #[arg(long)]
    pub dry_run: bool,

    /// Glob pattern to exclude pages by title
    #[arg(long)]
    pub exclude: Option<String>,
}

#[derive(Debug, Args)]
pub struct ConfluenceVersionDetailArgs {
    /// Page ID
    pub page_id: String,

    /// Version number
    pub version: u32,
}

#[derive(Debug, Args)]
pub struct ConfluencePageListArgs {
    /// Space IDs to filter by
    #[arg(long, short)]
    pub space_id: Option<Vec<String>>,

    /// Title to filter by
    #[arg(long, short)]
    pub title: Option<String>,

    /// Status filter (current, trashed, draft)
    #[arg(long)]
    pub status: Option<String>,

    /// Sort field
    #[arg(long)]
    pub sort: Option<String>,

    /// Max results
    #[arg(long, short, default_value = "25")]
    pub limit: u32,
}

#[derive(Debug, Args)]
pub struct ConfluenceUpdateTitleArgs {
    /// Page ID
    pub page_id: String,

    /// New title
    #[arg(long, short)]
    pub title: String,

    /// Version number
    #[arg(long)]
    pub version: u32,
}

#[derive(Debug, Args)]
pub struct ConfluencePageCustomContentArgs {
    /// Page ID
    pub page_id: String,

    /// Custom content type
    #[arg(long, short = 't')]
    pub content_type: String,

    /// Max results
    #[arg(long, short, default_value = "25")]
    pub limit: u32,
}

#[derive(Debug, Args)]
pub struct ConfluenceRedactArgs {
    /// Page ID
    pub page_id: String,
}
