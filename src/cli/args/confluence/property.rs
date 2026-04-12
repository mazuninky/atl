use clap::{Args, Subcommand};

use super::ConfluenceContentTypeIdArgs;

#[derive(Debug, Args)]
pub struct ConfluencePropertyCommand {
    #[command(subcommand)]
    pub command: ConfluencePropertySubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfluencePropertySubcommand {
    /// List all properties for a page
    List(ConfluencePropertyListArgs),

    /// Get a specific property
    Get(ConfluencePropertyGetArgs),

    /// Set (create or update) a property
    Set(ConfluencePropertySetArgs),

    /// Delete a property
    Delete(ConfluencePropertyDeleteArgs),
}

#[derive(Debug, Args)]
pub struct ConfluencePropertyListArgs {
    /// Page ID
    pub page_id: String,
}

#[derive(Debug, Args)]
pub struct ConfluencePropertyGetArgs {
    /// Page ID
    pub page_id: String,

    /// Property key
    pub key: String,
}

#[derive(Debug, Args)]
pub struct ConfluencePropertySetArgs {
    /// Page ID
    pub page_id: String,

    /// Property key
    pub key: String,

    /// Property value (JSON). Use @file to read from file, or - for stdin
    #[arg(long)]
    pub value: String,
}

#[derive(Debug, Args)]
pub struct ConfluencePropertyDeleteArgs {
    /// Page ID
    pub page_id: String,

    /// Property key
    pub key: String,
}

// =========================================================================
// Content-type properties (attached to whiteboards/databases/folders/etc.)
// =========================================================================

#[derive(Debug, Args)]
pub struct ConfluenceContentTypePropertyCommand {
    #[command(subcommand)]
    pub command: ConfluenceContentTypePropertySubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfluenceContentTypePropertySubcommand {
    /// List properties
    List(ConfluenceContentTypeIdArgs),

    /// Get a property by key
    Get(ConfluenceContentTypePropertyGetArgs),

    /// Set (create/update) a property
    Set(ConfluenceContentTypePropertySetArgs),

    /// Delete a property
    Delete(ConfluenceContentTypePropertyGetArgs),
}

#[derive(Debug, Args)]
pub struct ConfluenceContentTypePropertyGetArgs {
    /// Resource ID
    pub id: String,

    /// Property key
    pub key: String,
}

#[derive(Debug, Args)]
pub struct ConfluenceContentTypePropertySetArgs {
    /// Resource ID
    pub id: String,

    /// Property key
    pub key: String,

    /// Property value (JSON). Use @file to read from file, or - for stdin
    #[arg(long)]
    pub value: String,
}
