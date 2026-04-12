use clap::{Args, Subcommand};

/// Manage user-defined command aliases.
#[derive(Debug, Args)]
pub struct AliasCommand {
    #[command(subcommand)]
    pub command: AliasSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum AliasSubcommand {
    /// Create or update an alias
    Set(AliasSetArgs),

    /// List all configured aliases
    List,

    /// Delete an alias
    Delete(AliasDeleteArgs),
}

#[derive(Debug, Args)]
pub struct AliasSetArgs {
    /// Alias name (invoked as `atl <name>`)
    pub name: String,

    /// Expansion. Supports shell-style quoting, e.g.
    /// `'jira search "project=FOO"'`.
    pub expansion: String,
}

#[derive(Debug, Args)]
pub struct AliasDeleteArgs {
    /// Alias name to remove
    pub name: String,
}
