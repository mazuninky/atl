use clap::{Args, Subcommand};

// -- Jira Project Key --

#[derive(Debug, Args)]
pub struct JiraProjectKeyArgs {
    /// Project key (e.g. PROJ)
    pub project_key: String,
}

// -- Jira Project --

#[derive(Debug, Args)]
pub struct JiraProjectCommand {
    #[command(subcommand)]
    pub command: JiraProjectSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraProjectSubcommand {
    /// List all projects
    List,

    /// Get project details
    Get(JiraProjectKeyArgs),

    /// Create a project
    Create(JiraProjectCreateArgs),

    /// Update a project
    Update(JiraProjectUpdateArgs),

    /// Delete a project
    Delete(JiraProjectKeyArgs),

    /// List statuses for a project
    Statuses(JiraProjectKeyArgs),

    /// List roles for a project
    Roles(JiraProjectKeyArgs),

    /// Archive a project
    Archive(JiraProjectKeyArgs),

    /// Restore a project
    Restore(JiraProjectKeyArgs),

    /// List features for a project
    Features(JiraProjectKeyArgs),
}

#[derive(Debug, Args)]
pub struct JiraProjectCreateArgs {
    /// Project key (e.g. PROJ)
    #[arg(long, short)]
    pub key: String,

    /// Project name
    #[arg(long, short)]
    pub name: String,

    /// Project type key (e.g. software, business)
    #[arg(long, short = 't')]
    pub project_type_key: String,

    /// Lead account ID
    #[arg(long)]
    pub lead: String,

    /// Description
    #[arg(long, short)]
    pub description: Option<String>,

    /// Project template key
    #[arg(long)]
    pub template: Option<String>,
}

#[derive(Debug, Args)]
pub struct JiraProjectUpdateArgs {
    /// Project key
    pub key: String,

    /// New project name
    #[arg(long, short)]
    pub name: Option<String>,

    /// New lead account ID
    #[arg(long)]
    pub lead: Option<String>,

    /// New description
    #[arg(long, short)]
    pub description: Option<String>,
}
