use clap::{Args, Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum SprintState {
    Active,
    Closed,
    Future,
}

impl SprintState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Closed => "closed",
            Self::Future => "future",
        }
    }
}

#[derive(Debug, Args)]
pub struct JiraSprintsArgs {
    /// Board ID
    pub board_id: u64,

    /// Sprint state filter
    #[arg(long, short, value_enum)]
    pub state: Option<SprintState>,
}

#[derive(Debug, Args)]
pub struct JiraSprintIssuesArgs {
    /// Sprint ID
    pub sprint_id: u64,

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

#[derive(Debug, Args)]
pub struct JiraEpicCommand {
    #[command(subcommand)]
    pub command: JiraEpicSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraEpicSubcommand {
    /// List epics for a board
    List(JiraEpicListArgs),

    /// Get epic details
    Get(JiraEpicGetArgs),

    /// List issues in an epic
    Issues(JiraEpicIssuesArgs),

    /// Move issues into an epic
    Add(JiraEpicAddArgs),

    /// Remove issues from their epic
    Remove(JiraEpicRemoveArgs),
}

#[derive(Debug, Args)]
pub struct JiraEpicListArgs {
    /// Board ID
    pub board_id: u64,
}

#[derive(Debug, Args)]
pub struct JiraEpicGetArgs {
    /// Epic ID or issue key
    pub epic_id_or_key: String,
}

#[derive(Debug, Args)]
pub struct JiraEpicIssuesArgs {
    /// Epic ID or issue key
    pub epic_id_or_key: String,

    /// Max results per page
    #[arg(long, short, default_value = "50")]
    pub limit: u32,

    /// Fetch all results (auto-paginate)
    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct JiraEpicAddArgs {
    /// Epic issue key
    pub epic_key: String,

    /// Issue keys to add to the epic
    #[arg(required = true)]
    pub issues: Vec<String>,
}

#[derive(Debug, Args)]
pub struct JiraEpicRemoveArgs {
    /// Issue keys to remove from any epic
    #[arg(required = true)]
    pub issues: Vec<String>,
}

// -- Jira Sprint --

#[derive(Debug, Args)]
pub struct JiraSprintCommand {
    #[command(subcommand)]
    pub command: JiraSprintSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraSprintSubcommand {
    /// List sprints for a board
    List(JiraSprintsArgs),

    /// Get sprint details
    Get(JiraSprintIdArgs),

    /// List issues in a sprint
    Issues(JiraSprintIssuesArgs),

    /// Create a sprint
    Create(JiraSprintCreateArgs),

    /// Update a sprint
    Update(JiraSprintUpdateArgs),

    /// Delete a sprint
    Delete(JiraSprintIdArgs),

    /// Move issues to a sprint
    Move(JiraSprintMoveArgs),
}

#[derive(Debug, Args)]
pub struct JiraSprintIdArgs {
    /// Sprint ID
    pub sprint_id: u64,
}

#[derive(Debug, Args)]
pub struct JiraSprintCreateArgs {
    /// Board ID (origin board)
    #[arg(long, short)]
    pub board_id: u64,

    /// Sprint name
    #[arg(long, short)]
    pub name: String,

    /// Start date (ISO 8601, e.g. 2024-01-15T09:00:00.000Z)
    #[arg(long)]
    pub start_date: Option<String>,

    /// End date (ISO 8601)
    #[arg(long)]
    pub end_date: Option<String>,

    /// Sprint goal
    #[arg(long)]
    pub goal: Option<String>,
}

#[derive(Debug, Args)]
pub struct JiraSprintUpdateArgs {
    /// Sprint ID
    pub sprint_id: u64,

    /// New name
    #[arg(long, short)]
    pub name: Option<String>,

    /// New start date
    #[arg(long)]
    pub start_date: Option<String>,

    /// New end date
    #[arg(long)]
    pub end_date: Option<String>,

    /// New goal
    #[arg(long)]
    pub goal: Option<String>,

    /// New state (active, closed, future)
    #[arg(long, value_enum)]
    pub state: Option<SprintState>,
}

#[derive(Debug, Args)]
pub struct JiraSprintMoveArgs {
    /// Sprint ID
    pub sprint_id: u64,

    /// Issue keys to move into this sprint
    #[arg(required = true)]
    pub issues: Vec<String>,
}

// -- Jira Backlog --

#[derive(Debug, Args)]
pub struct JiraBacklogMoveArgs {
    /// Issue keys to move to backlog
    #[arg(required = true)]
    pub issues: Vec<String>,
}
