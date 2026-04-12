use clap::{Args, Subcommand};

use super::{JiraIdArgs, JiraProjectKeyArgs};

// -- Jira Component --

#[derive(Debug, Args)]
pub struct JiraComponentCommand {
    #[command(subcommand)]
    pub command: JiraComponentSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraComponentSubcommand {
    /// List components for a project
    List(JiraProjectKeyArgs),
    /// Get a component by ID
    Get(JiraComponentDeleteArgs),
    /// Create a component
    Create(JiraComponentCreateArgs),
    /// Update a component
    Update(JiraComponentUpdateArgs),
    /// Delete a component
    Delete(JiraComponentDeleteArgs),
}

#[derive(Debug, Args)]
pub struct JiraComponentUpdateArgs {
    /// Component ID
    pub id: String,
    /// New name
    #[arg(long, short)]
    pub name: Option<String>,
    /// New description
    #[arg(long, short)]
    pub description: Option<String>,
    /// New lead account ID
    #[arg(long)]
    pub lead: Option<String>,
    /// Assignee type
    #[arg(long)]
    pub assignee_type: Option<String>,
}

#[derive(Debug, Args)]
pub struct JiraComponentCreateArgs {
    /// Project key
    #[arg(long)]
    pub project: String,

    /// Component name
    #[arg(long, short)]
    pub name: String,

    /// Description
    #[arg(long, short)]
    pub description: Option<String>,

    /// Lead account ID
    #[arg(long)]
    pub lead: Option<String>,
}

#[derive(Debug, Args)]
pub struct JiraComponentDeleteArgs {
    /// Component ID
    pub id: String,
}

// -- Jira Dashboard --

#[derive(Debug, Args)]
pub struct JiraDashboardCommand {
    #[command(subcommand)]
    pub command: JiraDashboardSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraDashboardSubcommand {
    /// List dashboards
    List,

    /// Get a dashboard by ID
    Get(JiraDashboardGetArgs),

    /// Create a dashboard
    Create(JiraDashboardCreateArgs),

    /// Update a dashboard
    Update(JiraDashboardUpdateArgs),

    /// Delete a dashboard
    Delete(JiraDashboardGetArgs),

    /// Copy a dashboard
    Copy(JiraDashboardCopyArgs),

    /// List gadgets on a dashboard
    Gadgets(JiraDashboardGetArgs),

    /// Add a gadget to a dashboard
    AddGadget(JiraDashboardAddGadgetArgs),

    /// Update a gadget on a dashboard
    UpdateGadget(JiraDashboardUpdateGadgetArgs),

    /// Remove a gadget from a dashboard
    RemoveGadget(JiraDashboardGadgetArgs),
}

#[derive(Debug, Args)]
pub struct JiraDashboardGetArgs {
    /// Dashboard ID
    pub id: String,
}

#[derive(Debug, Args)]
pub struct JiraDashboardCreateArgs {
    /// Dashboard name
    #[arg(long, short)]
    pub name: String,

    /// Description
    #[arg(long, short)]
    pub description: Option<String>,
}

#[derive(Debug, Args)]
pub struct JiraDashboardUpdateArgs {
    /// Dashboard ID
    pub id: String,

    /// New name
    #[arg(long, short)]
    pub name: Option<String>,

    /// New description
    #[arg(long, short)]
    pub description: Option<String>,
}

#[derive(Debug, Args)]
pub struct JiraDashboardCopyArgs {
    /// Dashboard ID to copy
    pub id: String,

    /// Name for the copy
    #[arg(long, short)]
    pub name: Option<String>,
}

#[derive(Debug, Args)]
pub struct JiraDashboardAddGadgetArgs {
    /// Dashboard ID
    pub dashboard_id: String,

    /// Gadget URI
    #[arg(long)]
    pub uri: String,

    /// Gadget color
    #[arg(long)]
    pub color: Option<String>,

    /// Gadget position (row:column, e.g. "0:0")
    #[arg(long)]
    pub position: Option<String>,
}

#[derive(Debug, Args)]
pub struct JiraDashboardGadgetArgs {
    /// Dashboard ID
    pub dashboard_id: String,

    /// Gadget ID
    pub gadget_id: String,
}

#[derive(Debug, Args)]
pub struct JiraDashboardUpdateGadgetArgs {
    /// Dashboard ID
    pub dashboard_id: String,

    /// Gadget ID
    pub gadget_id: String,

    /// New gadget color
    #[arg(long)]
    pub color: Option<String>,

    /// New gadget position (row:column, e.g. "0:0")
    #[arg(long)]
    pub position: Option<String>,
}

// -- Jira Version --

#[derive(Debug, Args)]
pub struct JiraVersionCommand {
    #[command(subcommand)]
    pub command: JiraVersionSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraVersionSubcommand {
    /// List versions for a project
    List(JiraProjectKeyArgs),
    /// Get a version by ID
    Get(JiraVersionDeleteArgs),
    /// Create a version
    Create(JiraVersionCreateArgs),
    /// Update a version
    Update(JiraVersionUpdateArgs),
    /// Delete a version
    Delete(JiraVersionDeleteArgs),
    /// Mark a version as released
    Release(JiraVersionReleaseArgs),
}

#[derive(Debug, Args)]
pub struct JiraVersionUpdateArgs {
    /// Version ID
    pub id: String,
    /// New name
    #[arg(long, short)]
    pub name: Option<String>,
    /// New description
    #[arg(long, short)]
    pub description: Option<String>,
    /// Start date (YYYY-MM-DD)
    #[arg(long)]
    pub start_date: Option<String>,
    /// Release date (YYYY-MM-DD)
    #[arg(long)]
    pub release_date: Option<String>,
    /// Mark as released
    #[arg(long)]
    pub released: Option<bool>,
    /// Mark as archived
    #[arg(long)]
    pub archived: Option<bool>,
}

#[derive(Debug, Args)]
pub struct JiraVersionCreateArgs {
    /// Project key
    #[arg(long)]
    pub project: String,

    /// Version name
    #[arg(long, short)]
    pub name: String,

    /// Description
    #[arg(long, short)]
    pub description: Option<String>,

    /// Release date (YYYY-MM-DD)
    #[arg(long)]
    pub release_date: Option<String>,
}

#[derive(Debug, Args)]
pub struct JiraVersionDeleteArgs {
    /// Version ID
    pub id: String,
}

#[derive(Debug, Args)]
pub struct JiraVersionReleaseArgs {
    /// Version ID
    pub id: String,

    /// Release date (YYYY-MM-DD). Defaults to today if omitted.
    #[arg(long)]
    pub date: Option<String>,
}

// -- Jira Admin: Project Categories --

#[derive(Debug, Args)]
pub struct JiraProjectCategoryCommand {
    #[command(subcommand)]
    pub command: JiraProjectCategorySubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraProjectCategorySubcommand {
    /// List project categories
    List,
    /// Get a project category by ID
    Get(JiraIdArgs),
    /// Create a project category
    Create(JiraProjectCategoryCreateArgs),
    /// Update a project category
    Update(JiraProjectCategoryUpdateArgs),
    /// Delete a project category
    Delete(JiraIdArgs),
}

#[derive(Debug, Args)]
pub struct JiraProjectCategoryUpdateArgs {
    /// Category ID
    pub id: String,
    /// New name
    #[arg(long, short)]
    pub name: Option<String>,
    /// New description
    #[arg(long, short)]
    pub description: Option<String>,
}

#[derive(Debug, Args)]
pub struct JiraProjectCategoryCreateArgs {
    /// Category name
    #[arg(long, short)]
    pub name: String,

    /// Description
    #[arg(long, short)]
    pub description: Option<String>,
}

// -- Jira Admin: Webhooks --

#[derive(Debug, Args)]
pub struct JiraWebhookCommand {
    #[command(subcommand)]
    pub command: JiraWebhookSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraWebhookSubcommand {
    /// List webhooks
    List,

    /// Get a webhook by ID
    Get(JiraIdArgs),

    /// Create a webhook
    Create(JiraWebhookCreateArgs),

    /// Delete a webhook
    Delete(JiraIdArgs),
}

#[derive(Debug, Args)]
pub struct JiraWebhookCreateArgs {
    /// Webhook name
    #[arg(long, short)]
    pub name: String,

    /// Webhook URL
    #[arg(long, short)]
    pub url: String,

    /// Events (comma-separated, e.g. "jira:issue_created,jira:issue_updated")
    #[arg(long, short)]
    pub events: String,

    /// JQL filter
    #[arg(long)]
    pub jql: Option<String>,
}

// -- Jira Admin: Audit Records --

#[derive(Debug, Args)]
pub struct JiraAuditRecordsArgs {
    /// Max results
    #[arg(long, short, default_value = "100")]
    pub limit: u32,

    /// Offset
    #[arg(long, default_value = "0")]
    pub offset: u32,

    /// Filter text
    #[arg(long, short)]
    pub filter: Option<String>,

    /// From date (ISO 8601)
    #[arg(long)]
    pub from: Option<String>,

    /// To date (ISO 8601)
    #[arg(long)]
    pub to: Option<String>,
}

// -- Jira Admin: Link Types --

#[derive(Debug, Args)]
pub struct JiraLinkTypeCommand {
    #[command(subcommand)]
    pub command: JiraLinkTypeSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraLinkTypeSubcommand {
    /// List all issue link types
    List,
    /// Get an issue link type by ID
    Get(JiraIdArgs),
    /// Create an issue link type
    Create(JiraLinkTypeCreateArgs),
    /// Update an issue link type
    Update(JiraLinkTypeUpdateArgs),
    /// Delete an issue link type
    Delete(JiraIdArgs),
}

#[derive(Debug, Args)]
pub struct JiraLinkTypeCreateArgs {
    /// Link type name
    #[arg(long, short)]
    pub name: String,
    /// Inward description (e.g. "is blocked by")
    #[arg(long)]
    pub inward: String,
    /// Outward description (e.g. "blocks")
    #[arg(long)]
    pub outward: String,
}

#[derive(Debug, Args)]
pub struct JiraLinkTypeUpdateArgs {
    /// Link type ID
    pub id: String,
    /// New name
    #[arg(long, short)]
    pub name: Option<String>,
    /// New inward description
    #[arg(long)]
    pub inward: Option<String>,
    /// New outward description
    #[arg(long)]
    pub outward: Option<String>,
}

// -- Jira Admin: Standalone Roles --

#[derive(Debug, Args)]
pub struct JiraRoleCommand {
    #[command(subcommand)]
    pub command: JiraRoleSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraRoleSubcommand {
    /// List all roles
    List,
    /// Get a role by ID
    Get(JiraIdArgs),
    /// Create a role
    Create(JiraRoleCreateArgs),
    /// Delete a role
    Delete(JiraIdArgs),
}

#[derive(Debug, Args)]
pub struct JiraRoleCreateArgs {
    /// Role name
    #[arg(long, short)]
    pub name: String,
    /// Description
    #[arg(long, short)]
    pub description: Option<String>,
}

// -- Jira Admin: Banner --

#[derive(Debug, Args)]
pub struct JiraBannerCommand {
    #[command(subcommand)]
    pub command: JiraBannerSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraBannerSubcommand {
    /// Get the announcement banner
    Get,
    /// Set the announcement banner
    Set(JiraBannerSetArgs),
}

#[derive(Debug, Args)]
pub struct JiraBannerSetArgs {
    /// Banner message (HTML)
    #[arg(long, short)]
    pub message: String,
    /// Enable or disable the banner
    #[arg(long)]
    pub is_enabled: Option<bool>,
    /// Visibility: "public" or "private"
    #[arg(long)]
    pub visibility: Option<String>,
}

// -- Jira Admin: Async Tasks --

#[derive(Debug, Args)]
pub struct JiraTaskCommand {
    #[command(subcommand)]
    pub command: JiraTaskSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraTaskSubcommand {
    /// Get an async task by ID
    Get(JiraIdArgs),
    /// Cancel an async task
    Cancel(JiraIdArgs),
}

// -- Jira Admin: Attachment Admin --

#[derive(Debug, Args)]
pub struct JiraAttachmentAdminCommand {
    #[command(subcommand)]
    pub command: JiraAttachmentAdminSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraAttachmentAdminSubcommand {
    /// Get an attachment by ID
    Get(JiraIdArgs),
    /// Delete an attachment
    Delete(JiraIdArgs),
    /// Get attachment upload metadata/settings
    Meta,
}
