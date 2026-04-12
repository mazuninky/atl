use clap::{Args, Subcommand};

#[derive(Debug, Args)]
pub struct ConfluenceLabelCommand {
    #[command(subcommand)]
    pub command: ConfluenceLabelSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfluenceLabelSubcommand {
    /// List labels for a page
    List(ConfluenceLabelListArgs),

    /// Add labels to a page
    Add(ConfluenceLabelAddArgs),

    /// Remove a label from a page
    Remove(ConfluenceLabelRemoveArgs),

    /// List pages for a label
    Pages(ConfluenceLabelIdLimitArgs),

    /// List blog posts for a label
    Blogposts(ConfluenceLabelIdLimitArgs),

    /// List attachments for a label
    Attachments(ConfluenceLabelIdLimitArgs),
}

#[derive(Debug, Args)]
pub struct ConfluenceLabelListArgs {
    /// Page ID
    pub page_id: String,

    /// Filter by label prefix (e.g. "global", "my", "team")
    #[arg(long)]
    pub prefix: Option<String>,
}

#[derive(Debug, Args)]
pub struct ConfluenceLabelAddArgs {
    /// Page ID
    pub page_id: String,

    /// Labels to add
    #[arg(required = true)]
    pub labels: Vec<String>,
}

#[derive(Debug, Args)]
pub struct ConfluenceLabelRemoveArgs {
    /// Page ID
    pub page_id: String,

    /// Label to remove
    pub label: String,
}

#[derive(Debug, Args)]
pub struct ConfluenceLabelIdLimitArgs {
    /// Label ID
    pub label_id: String,

    /// Max results
    #[arg(long, short, default_value = "25")]
    pub limit: u32,
}
