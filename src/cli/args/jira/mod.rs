use clap::{Args, Subcommand};

mod admin;
mod board;
mod field;
mod filter;
mod issue;
mod project;
mod sprint;
mod user;
mod workflow;

pub use admin::*;
pub use board::*;
pub use field::*;
pub use filter::*;
pub use issue::*;
pub use project::*;
pub use sprint::*;
pub use user::*;
pub use workflow::*;

// -- Jira --

#[derive(Debug, Args)]
pub struct JiraCommand {
    #[command(subcommand)]
    pub command: JiraSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum JiraSubcommand {
    /// Search issues with JQL
    Search(JiraSearchArgs),

    /// View an issue
    View(JiraViewArgs),

    /// Create a new issue
    Create(JiraCreateArgs),

    /// Update an issue
    Update(JiraUpdateArgs),

    /// Delete an issue
    Delete(JiraDeleteArgs),

    /// Transition an issue to another status
    Move(JiraMoveArgs),

    /// Assign an issue
    Assign(JiraAssignArgs),

    /// Add a comment to an issue
    Comment(JiraCommentArgs),

    /// List comments for an issue
    Comments(JiraIssueKeyArgs),

    /// Get a specific comment
    CommentGet(JiraCommentGetArgs),

    /// Delete a comment
    CommentDelete(JiraCommentDeleteArgs),

    /// List available transitions for an issue
    Transitions(JiraIssueKeyArgs),

    /// Project management
    Project(JiraProjectCommand),

    /// Board management
    Board(JiraBoardCommand),

    /// Sprint management
    Sprint(JiraSprintCommand),

    /// Move issues to backlog
    BacklogMove(JiraBacklogMoveArgs),

    /// Show current user info
    Me,

    /// Epic management
    Epic(JiraEpicCommand),

    /// Link two issues
    Link(JiraLinkArgs),

    /// Issue link type management
    LinkType(JiraLinkTypeCommand),

    /// Get an issue link by ID
    IssueLinkGet(JiraIdArgs),

    /// Delete an issue link by ID
    IssueLinkDelete(JiraIdArgs),

    /// Add a remote link to an issue
    RemoteLink(JiraRemoteLinkAddArgs),

    /// List remote links for an issue
    RemoteLinks(JiraIssueKeyArgs),

    /// Delete a remote link
    RemoteLinkDelete(JiraRemoteLinkDeleteArgs),

    /// Clone an issue
    Clone(JiraCloneArgs),

    /// Worklog management
    Worklog(JiraWorklogCommand),

    /// Saved filter management
    Filter(JiraFilterCommand),

    /// Attach a file to an issue
    Attach(JiraAttachArgs),

    /// Watch an issue
    Watch(JiraIssueKeyArgs),

    /// Unwatch an issue
    Unwatch(JiraIssueKeyArgs),

    /// List watchers for an issue
    Watchers(JiraIssueKeyArgs),

    /// Vote for an issue
    Vote(JiraIssueKeyArgs),

    /// Remove your vote from an issue
    Unvote(JiraIssueKeyArgs),

    /// View issue changelog (history of changes)
    Changelog(JiraChangelogArgs),

    /// Component management
    Component(JiraComponentCommand),

    /// Version management
    Version(JiraVersionCommand),

    /// Dashboard management
    Dashboard(JiraDashboardCommand),

    /// Field management
    Field(JiraFieldCommand),

    /// User operations
    User(JiraUserCommand),

    /// Group management
    Group(JiraGroupCommand),

    /// Send a notification about an issue
    Notify(JiraNotifyArgs),

    /// Get issue creation metadata
    CreateMeta(JiraCreateMetaArgs),

    /// Get issue edit metadata
    EditMeta(JiraIssueKeyArgs),

    /// Issue type management
    IssueType(JiraIssueTypeCommand),

    /// Priority management
    Priority(JiraPriorityCommand),

    /// Resolution management
    Resolution(JiraResolutionCommand),

    /// Status management
    Status(JiraStatusCommand),

    /// Screen management
    Screen(JiraScreenCommand),

    /// Workflow management
    Workflow(JiraWorkflowCommand),

    /// Workflow scheme management
    WorkflowScheme(JiraWorkflowSchemeCommand),

    /// Permission scheme management
    PermissionScheme(JiraPermissionSchemeCommand),

    /// Notification scheme management
    NotificationScheme(JiraNotificationSchemeCommand),

    /// Issue security scheme management
    IssueSecurityScheme(JiraIssueSecuritySchemeCommand),

    /// Field configuration management
    FieldConfig(JiraFieldConfigCommand),

    /// Issue type scheme management
    IssueTypeScheme(JiraIssueTypeSchemeCommand),

    /// Standalone role management
    Role(JiraRoleCommand),

    /// Announcement banner
    Banner(JiraBannerCommand),

    /// View system configuration
    Configuration,

    /// Async task management
    Task(JiraTaskCommand),

    /// Attachment administration
    Attachment(JiraAttachmentAdminCommand),

    /// Project category management
    ProjectCategory(JiraProjectCategoryCommand),

    /// Show server information
    ServerInfo,

    /// Webhook management
    Webhook(JiraWebhookCommand),

    /// View audit records
    AuditRecords(JiraAuditRecordsArgs),

    /// List all permissions
    Permissions,

    /// List my permissions
    MyPermissions,

    /// List all labels
    Labels(JiraLabelsArgs),
}
