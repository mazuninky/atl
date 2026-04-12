use clap::{Args, Subcommand};

#[derive(Debug, Args)]
pub struct JiraBoardsArgs {
    /// Filter by project key
    #[arg(long)]
    pub project: Option<String>,
}

// -- Jira Board --

#[derive(Debug, Args)]
pub struct JiraBoardCommand {
    #[command(subcommand)]
    pub command: JiraBoardSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraBoardSubcommand {
    /// List boards
    List(JiraBoardsArgs),

    /// Get board details
    Get(JiraBoardIdArgs),

    /// Get board configuration
    Config(JiraBoardIdArgs),

    /// List all issues on a board
    Issues(JiraBoardIssuesArgs),

    /// List backlog issues for a board
    Backlog(JiraBoardIssuesArgs),
}

#[derive(Debug, Args)]
pub struct JiraBoardIdArgs {
    /// Board ID
    pub board_id: u64,
}

#[derive(Debug, Args)]
pub struct JiraBoardIssuesArgs {
    /// Board ID
    pub board_id: u64,

    /// Max results per page
    #[arg(long, short, default_value = "50")]
    pub limit: u32,

    /// Fetch all results (auto-paginate)
    #[arg(long)]
    pub all: bool,

    /// Fields to return (comma-separated)
    #[arg(long, short, default_value = "key,summary,status,assignee")]
    pub fields: String,
}
