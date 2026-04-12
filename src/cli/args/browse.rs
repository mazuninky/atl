use clap::{Args, ValueEnum};

/// Arguments for `atl browse`.
#[derive(Debug, Args)]
pub struct BrowseArgs {
    /// Page ID or Jira issue key to open in a browser.
    pub target: String,

    /// Which service to target.
    ///
    /// When `auto`, Jira-style keys (`PROJ-123`) route to Jira and
    /// everything else routes to Confluence.
    #[arg(long, value_enum, default_value_t = BrowseService::Auto)]
    pub service: BrowseService,
}

/// Selects which Atlassian service `atl browse` resolves against.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum BrowseService {
    /// Auto-detect from the target's shape.
    Auto,
    /// Force Confluence.
    Confluence,
    /// Force Jira.
    Jira,
}
