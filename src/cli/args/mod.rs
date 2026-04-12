mod alias;
pub mod api;
mod auth;
mod browse;
mod confluence;
mod jira;
mod updater;

pub use alias::*;
pub use api::*;
pub use auth::*;
pub use browse::*;
pub use confluence::*;
pub use jira::*;
pub use updater::*;

use camino::Utf8PathBuf;
use clap::{Args, Parser, Subcommand};

use crate::output::OutputFormat;

/// Unified CLI for Atlassian Confluence and Jira.
#[derive(Debug, Parser)]
#[command(name = "atl")]
#[command(author, version, about, long_about = None)]
// Subcommands define their own `--version` argument (content version in
// Confluence update endpoints), so we suppress the auto-generated
// `--version` flag globally to avoid clap debug-assertion collisions.
#[command(disable_version_flag = true)]
pub struct Cli {
    /// Increase logging verbosity (-v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Suppress all output except errors
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    /// Path to configuration file
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<Utf8PathBuf>,

    /// Profile name to use
    #[arg(short, long, global = true, env = "ATL_PROFILE")]
    pub profile: Option<String>,

    /// Output format
    #[arg(
        long,
        short = 'F',
        global = true,
        default_value = "console",
        value_enum
    )]
    pub format: OutputFormat,

    /// Disable colored output
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Do not pipe long output through a pager
    #[arg(long, global = true)]
    pub no_pager: bool,

    /// Query output with a jq expression
    #[arg(long, global = true, value_name = "EXPR")]
    pub jq: Option<String>,

    /// Format output with a minijinja template
    #[arg(long, global = true, value_name = "TEMPLATE")]
    pub template: Option<String>,

    /// Maximum HTTP retries on transient errors (0 = off)
    #[arg(long, global = true, default_value_t = 3)]
    pub retries: u32,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Confluence operations
    #[command(alias = "conf", alias = "c")]
    Confluence(ConfluenceCommand),

    /// Jira operations
    #[command(alias = "j")]
    Jira(Box<JiraCommand>),

    /// Initialize configuration file
    Init,

    /// Manage configuration profiles
    Config(ConfigCommand),

    /// Generate shell completion scripts
    Completions(CompletionsArgs),

    /// Manage the atl binary itself (check/update)
    #[command(name = "self")]
    Self_(SelfCommand),

    /// Make an authenticated REST request against Confluence or Jira
    Api(Box<ApiArgs>),

    /// Open a Confluence page or Jira issue in the default browser
    Browse(BrowseArgs),

    /// Manage user-defined command aliases
    Alias(AliasCommand),

    /// Manage authentication — login, logout, status, token
    Auth(AuthCommand),

    /// Generate man pages, shell completions, and markdown reference (hidden)
    #[command(hide = true)]
    GenerateDocs(GenerateDocsArgs),
}

#[derive(Debug, Args)]
pub struct GenerateDocsArgs {
    /// Output directory (will be created if missing)
    #[arg(long, value_name = "DIR")]
    pub output_dir: Utf8PathBuf,
}

#[derive(Debug, Args)]
pub struct ConfigCommand {
    #[command(subcommand)]
    pub command: ConfigSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfigSubcommand {
    /// List all profiles
    List,

    /// Show profile details
    Show(ConfigShowArgs),

    /// Delete a profile
    Delete(ConfigDeleteArgs),

    /// Set the default profile
    SetDefault(ConfigSetDefaultArgs),

    /// Set default project/workspace for a profile
    SetDefaults(ConfigSetDefaultsArgs),
}

#[derive(Debug, Args)]
pub struct ConfigShowArgs {
    /// Profile name (uses default if omitted)
    pub name: Option<String>,
}

#[derive(Debug, Args)]
pub struct ConfigDeleteArgs {
    /// Profile name to delete
    pub name: String,
}

#[derive(Debug, Args)]
pub struct ConfigSetDefaultArgs {
    /// Profile name to set as default
    pub name: String,
}

#[derive(Debug, Args)]
#[command(group(
    clap::ArgGroup::new("defaults")
        .required(true)
        .args(["project", "space"])
))]
pub struct ConfigSetDefaultsArgs {
    /// Profile name (uses default if omitted)
    pub profile: Option<String>,

    /// Default Jira project key
    #[arg(long)]
    pub project: Option<String>,

    /// Default Confluence space key
    #[arg(long)]
    pub space: Option<String>,
}

#[derive(Debug, Args)]
pub struct CompletionsArgs {
    /// Shell to generate completions for
    #[arg(value_enum)]
    pub shell: clap_complete::Shell,
}
