use clap::{Args, Subcommand};

/// Top-level `atl self` command — manages the atl binary itself.
#[derive(Debug, Args)]
pub struct SelfCommand {
    #[command(subcommand)]
    pub command: SelfSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum SelfSubcommand {
    /// Check for a newer release without downloading
    Check,
    /// Download and replace the current binary with the latest release
    Update(SelfUpdateArgs),
}

#[derive(Debug, Args)]
pub struct SelfUpdateArgs {
    /// Install a specific version instead of latest (e.g. 2026.16.2)
    #[arg(long, value_name = "VERSION")]
    pub to: Option<String>,

    /// Permit downgrading when --to points at an older version
    #[arg(long, requires = "to")]
    pub allow_downgrade: bool,
}
