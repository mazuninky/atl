//! clap arg definitions for `atl auth`.
//!
//! The subcommand tree is intentionally small — four verbs
//! (`login`/`logout`/`status`/`token`) that all share the same
//! `--service`/`--profile` selectors. See the handler in
//! `src/cli/commands/auth.rs` for behaviour.

use clap::{Args, Subcommand, ValueEnum};

/// Top-level `atl auth` args.
#[derive(Debug, Args)]
pub struct AuthCommand {
    #[command(subcommand)]
    pub command: AuthSubcommand,
}

/// Which Atlassian product the auth action applies to.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AuthService {
    /// Only Confluence.
    Confluence,
    /// Only Jira.
    Jira,
    /// Both Confluence and Jira (the default).
    #[default]
    Both,
}

/// Auth mechanism for a given instance.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AuthKind {
    /// Basic auth — email + API token, the usual Atlassian Cloud choice.
    #[default]
    Basic,
    /// Bearer auth — Personal Access Token, used by Atlassian Data Center.
    Bearer,
}

/// Narrow service selector used by the `token` subcommand, which must pick
/// exactly one service (unlike login/logout/status which accept `both`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SingleService {
    /// Confluence only.
    Confluence,
    /// Jira only.
    #[default]
    Jira,
}

/// `atl auth` verbs.
#[derive(Debug, Subcommand)]
pub enum AuthSubcommand {
    /// Log in to Confluence or Jira and store the token in the OS keyring
    Login(AuthLoginArgs),

    /// Remove stored credentials for a profile/service
    Logout(AuthLogoutArgs),

    /// Show which profiles/services are authenticated
    Status(AuthStatusArgs),

    /// Print the resolved API token to stdout
    Token(AuthTokenArgs),
}

#[derive(Debug, Args)]
pub struct AuthLoginArgs {
    /// Service to log in to (defaults to both)
    #[arg(long, value_enum, default_value_t = AuthService::default())]
    pub service: AuthService,

    /// Profile name (overrides the global --profile for this login)
    #[arg(long)]
    pub profile: Option<String>,

    /// Atlassian domain (e.g. acme.atlassian.net). Prompted when omitted.
    #[arg(long)]
    pub domain: Option<String>,

    /// Email address for Basic auth. Prompted when omitted and --auth-type=basic.
    #[arg(long)]
    pub email: Option<String>,

    /// Authentication mechanism (basic = email + API token, bearer = PAT)
    #[arg(long, value_enum, default_value_t = AuthKind::default())]
    pub auth_type: AuthKind,

    /// Read the token from stdin instead of prompting interactively
    #[arg(long)]
    pub with_token: bool,

    /// Skip the live verification request against the Atlassian API
    #[arg(long)]
    pub skip_verify: bool,
}

#[derive(Debug, Args)]
pub struct AuthLogoutArgs {
    /// Service to log out of (defaults to both)
    #[arg(long, value_enum, default_value_t = AuthService::default())]
    pub service: AuthService,

    /// Profile name (overrides the global --profile for this logout)
    #[arg(long)]
    pub profile: Option<String>,
}

#[derive(Debug, Args)]
pub struct AuthStatusArgs {
    /// Show status for this profile only (defaults to all)
    #[arg(long)]
    pub profile: Option<String>,

    /// Skip the live verification request against the Atlassian API
    #[arg(long)]
    pub skip_verify: bool,
}

#[derive(Debug, Args)]
pub struct AuthTokenArgs {
    /// Which service's token to print (required)
    #[arg(long, value_enum, default_value_t = SingleService::default())]
    pub service: SingleService,

    /// Profile name (overrides the global --profile)
    #[arg(long)]
    pub profile: Option<String>,

    /// Print the token even when stdout is a TTY
    #[arg(long)]
    pub force: bool,
}
