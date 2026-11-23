//! `atl skill` — the agent skill of this atl (`install`, `update`, `status`,
//! `remove`).

use clap::{Args, Subcommand};

/// Top-level `atl skill` command.
#[derive(Debug, Args)]
pub struct SkillCommand {
    /// Operation to perform on the skill.
    #[command(subcommand)]
    pub command: SkillSubcommand,
}

/// Subcommands of `atl skill`.
#[derive(Debug, Subcommand)]
pub enum SkillSubcommand {
    /// Write the skill, and the link to it for Claude Code
    Install(SkillWriteArgs),
    /// Bring an installed skill to the version of this atl
    Update(SkillWriteArgs),
    /// Say where the skill is, of which version, and whether it was changed
    Status(SkillStatusArgs),
    /// Remove the files atl installed, and the link to them
    Remove(SkillWriteArgs),
}

/// Arguments for `atl skill install`, `update` and `remove`.
#[derive(Debug, Args)]
pub struct SkillWriteArgs {
    /// Work on the skill of the current directory (`.agents/skills/atl`)
    /// instead of the home directory's
    #[arg(long)]
    pub project: bool,

    /// Go ahead where files were changed since atl wrote them, where others
    /// are in their place, or where atl did not install the directory: atl's
    /// own files are replaced (or removed), every other file stays.
    ///
    /// Not overridden: a skill directory that is a symbolic link, a manifest
    /// of a newer atl, a directory in the place of a file of the skill, a
    /// file or a directory in the place of Claude Code's link (another link
    /// there is replaced), root in the home directory of another user.
    #[arg(long)]
    pub force: bool,
}

/// Arguments for `atl skill status`.
#[derive(Debug, Args)]
pub struct SkillStatusArgs {
    /// Look at the skill of the current directory (`.agents/skills/atl`)
    /// instead of the home directory's
    #[arg(long)]
    pub project: bool,
}
