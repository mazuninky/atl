mod admin;
mod board;
mod field;
mod filter;
mod project;
mod sprint;
mod user;
mod workflow;

use camino::Utf8Path;
use serde_json::{Value, json};

use crate::cli::args::*;
use crate::client::JiraClient;
use crate::config::ConfigLoader;
use crate::io::IoStreams;
use crate::output::{OutputFormat, Transforms, write_output};

use super::read_body_arg;

pub(super) fn today_date() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let days = now / 86400;
    // Algorithm to convert days since epoch to YYYY-MM-DD
    let (y, m, d) = civil_from_days(days as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    // Howard Hinnant's algorithm
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

pub(super) fn insert_extra_fields(
    map: &mut serde_json::Map<String, Value>,
    fix_version: &Option<String>,
    component: &Option<String>,
    custom_fields: &[String],
) -> anyhow::Result<()> {
    if let Some(fv) = fix_version {
        let versions: Vec<Value> = fv.split(',').map(|v| json!({ "name": v.trim() })).collect();
        map.insert("fixVersions".into(), Value::Array(versions));
    }
    if let Some(comp) = component {
        let components: Vec<Value> = comp
            .split(',')
            .map(|c| json!({ "name": c.trim() }))
            .collect();
        map.insert("components".into(), Value::Array(components));
    }
    for entry in custom_fields {
        let (key, raw_val) = entry.split_once('=').ok_or_else(|| {
            anyhow::anyhow!("invalid --custom format: expected KEY=VALUE, got '{entry}'")
        })?;
        let val = serde_json::from_str(raw_val).unwrap_or(Value::String(raw_val.to_string()));
        map.insert(key.to_string(), val);
    }
    Ok(())
}

/// Escape a value for safe interpolation into a JQL quoted string.
fn escape_jql(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn build_jql(args: &JiraSearchArgs) -> anyhow::Result<String> {
    let mut clauses = Vec::new();

    if let Some(jql) = &args.jql {
        clauses.push(format!("({jql})"));
    }
    if let Some(v) = &args.status {
        clauses.push(format!("status = \"{}\"", escape_jql(v)));
    }
    if let Some(v) = &args.priority {
        clauses.push(format!("priority = \"{}\"", escape_jql(v)));
    }
    if let Some(v) = &args.assignee {
        if v == "currentUser()" {
            clauses.push("assignee = currentUser()".to_string());
        } else {
            clauses.push(format!("assignee = \"{}\"", escape_jql(v)));
        }
    }
    if let Some(v) = &args.reporter {
        if v == "currentUser()" {
            clauses.push("reporter = currentUser()".to_string());
        } else {
            clauses.push(format!("reporter = \"{}\"", escape_jql(v)));
        }
    }
    if let Some(v) = &args.r#type {
        clauses.push(format!("type = \"{}\"", escape_jql(v)));
    }
    if let Some(v) = &args.label {
        clauses.push(format!("labels = \"{}\"", escape_jql(v)));
    }
    if let Some(v) = &args.component {
        clauses.push(format!("component = \"{}\"", escape_jql(v)));
    }
    if let Some(v) = &args.resolution {
        clauses.push(format!("resolution = \"{}\"", escape_jql(v)));
    }
    if let Some(v) = &args.created {
        clauses.push(format!("created >= \"{}\"", escape_jql(v)));
    }
    if let Some(v) = &args.created_after {
        clauses.push(format!("created > \"{}\"", escape_jql(v)));
    }
    if let Some(v) = &args.updated {
        clauses.push(format!("updated >= \"{}\"", escape_jql(v)));
    }
    if let Some(v) = &args.updated_after {
        clauses.push(format!("updated > \"{}\"", escape_jql(v)));
    }
    if args.watching {
        clauses.push("watcher = currentUser()".to_string());
    }

    if clauses.is_empty() {
        anyhow::bail!("provide a JQL query or at least one filter flag");
    }

    let mut jql = clauses.join(" AND ");

    if let Some(field) = &args.order_by {
        let dir = if args.reverse { "DESC" } else { "ASC" };
        jql.push_str(&format!(" ORDER BY {field} {dir}"));
    }

    Ok(jql)
}

pub async fn run(
    cmd: &JiraSubcommand,
    config_path: Option<&Utf8Path>,
    profile_name: Option<&str>,
    retries: u32,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    // `--web` short-circuits to the browse helper so we don't hit the Jira
    // API at all — the URL is derived from the configured domain + key.
    if let JiraSubcommand::View(args) = cmd
        && args.web
    {
        let browse_args = crate::cli::args::BrowseArgs {
            target: args.key.clone(),
            service: crate::cli::args::BrowseService::Jira,
        };
        return super::browse::run(&browse_args, config_path, profile_name, retries, io).await;
    }

    let config = ConfigLoader::load(config_path)?;
    let profile = config
        .as_ref()
        .and_then(|c| c.resolve_profile(profile_name))
        .ok_or_else(|| {
            crate::error::Error::Config("no profile found; run `atl init` first".into())
        })?;
    let instance = profile.jira.as_ref().ok_or_else(|| {
        crate::error::Error::Config("no Jira instance configured in profile".into())
    })?;
    let client = JiraClient::new(instance, retries)?;

    dispatch(cmd, &client, format, io, transforms).await
}

/// Returns true when the long-form output of `cmd` would benefit from a
/// pager. Only the read-heavy "view" commands qualify.
fn cmd_uses_pager(cmd: &JiraSubcommand) -> bool {
    matches!(cmd, JiraSubcommand::View(_) | JiraSubcommand::Search(_))
}

async fn dispatch(
    cmd: &JiraSubcommand,
    client: &JiraClient,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    let value = match cmd {
        JiraSubcommand::Search(args) => {
            let jql = build_jql(args)?;
            let fields: Vec<&str> = args.fields.split(',').map(str::trim).collect();
            if args.all {
                client.search_issues_all(&jql, args.limit, &fields).await?
            } else {
                client.search_issues(&jql, args.limit, &fields).await?
            }
        }
        JiraSubcommand::View(args) => client.get_issue(&args.key, &[]).await?,
        JiraSubcommand::Create(args) => {
            let mut fields = json!({
                "project": { "key": &args.project },
                "issuetype": { "name": &args.issue_type },
                "summary": &args.summary,
            });
            if let Some(map) = fields.as_object_mut() {
                if let Some(desc) = &args.description {
                    let body = read_body_arg(desc)?;
                    map.insert("description".into(), Value::String(body));
                }
                if let Some(assignee) = &args.assignee {
                    map.insert("assignee".into(), json!({ "accountId": assignee }));
                }
                if let Some(priority) = &args.priority {
                    map.insert("priority".into(), json!({ "name": priority }));
                }
                if let Some(labels) = &args.labels {
                    let label_list: Vec<Value> = labels
                        .split(',')
                        .map(|l| Value::String(l.trim().to_string()))
                        .collect();
                    map.insert("labels".into(), Value::Array(label_list));
                }
                if let Some(parent) = &args.parent {
                    map.insert("parent".into(), json!({ "key": parent }));
                }
                insert_extra_fields(map, &args.fix_version, &args.component, &args.custom_fields)?;
            }
            client.create_issue(&json!({ "fields": fields })).await?
        }
        JiraSubcommand::Update(args) => {
            let mut fields = serde_json::Map::new();
            if let Some(summary) = &args.summary {
                fields.insert("summary".into(), Value::String(summary.clone()));
            }
            if let Some(desc) = &args.description {
                let body = read_body_arg(desc)?;
                fields.insert("description".into(), Value::String(body));
            }
            if let Some(assignee) = &args.assignee {
                fields.insert("assignee".into(), json!({ "accountId": assignee }));
            }
            if let Some(priority) = &args.priority {
                fields.insert("priority".into(), json!({ "name": priority }));
            }
            if let Some(labels) = &args.labels {
                let label_list: Vec<Value> = labels
                    .split(',')
                    .map(|l| Value::String(l.trim().to_string()))
                    .collect();
                fields.insert("labels".into(), Value::Array(label_list));
            }
            insert_extra_fields(
                &mut fields,
                &args.fix_version,
                &args.component,
                &args.custom_fields,
            )?;
            if fields.is_empty() {
                anyhow::bail!(
                    "no fields to update; specify at least one of --summary, --description, --assignee, --priority, --labels, --fix-version, --component, --custom"
                );
            }
            client
                .update_issue(&args.key, &json!({ "fields": fields }))
                .await?
        }
        JiraSubcommand::Delete(args) => {
            client.delete_issue(&args.key, args.delete_subtasks).await?;
            Value::String(format!("Issue {} deleted", args.key))
        }
        JiraSubcommand::Move(args) => client.transition_issue(&args.key, &args.transition).await?,
        JiraSubcommand::Assign(args) => {
            client.assign_issue(&args.key, &args.account_id).await?;
            Value::String(format!("Issue {} assigned", args.key))
        }
        JiraSubcommand::Comment(args) => {
            let body = read_body_arg(&args.body)?;
            client.add_comment(&args.key, &body).await?
        }
        JiraSubcommand::Comments(args) => client.list_comments(&args.key).await?,
        JiraSubcommand::CommentGet(args) => client.get_comment(&args.key, &args.comment_id).await?,
        JiraSubcommand::CommentDelete(args) => {
            client.delete_comment(&args.key, &args.comment_id).await?;
            Value::String(format!(
                "Comment {} deleted from {}",
                args.comment_id, args.key
            ))
        }
        JiraSubcommand::Transitions(args) => client.get_transitions(&args.key).await?,
        JiraSubcommand::Project(cmd) => project::dispatch_project(&cmd.command, client).await?,
        JiraSubcommand::Board(cmd) => board::dispatch_board(&cmd.command, client).await?,
        JiraSubcommand::Sprint(cmd) => sprint::dispatch_sprint(&cmd.command, client).await?,
        JiraSubcommand::BacklogMove(args) => {
            client.move_issues_to_backlog(&args.issues).await?;
            Value::String("Issues moved to backlog".to_string())
        }
        JiraSubcommand::Me => client.get_myself().await?,
        JiraSubcommand::Epic(epic_cmd) => sprint::dispatch_epic(&epic_cmd.command, client).await?,
        JiraSubcommand::Link(args) => {
            client
                .create_issue_link(&args.link_type, &args.inward_key, &args.outward_key)
                .await?;
            Value::String(format!(
                "Linked {} -> {} ({})",
                args.inward_key, args.outward_key, args.link_type
            ))
        }
        JiraSubcommand::LinkType(cmd) => admin::dispatch_link_type(&cmd.command, client).await?,
        JiraSubcommand::IssueLinkGet(args) => client.get_issue_link(&args.id).await?,
        JiraSubcommand::IssueLinkDelete(args) => {
            client.delete_issue_link(&args.id).await?;
            Value::String(format!("Issue link {} deleted", args.id))
        }
        JiraSubcommand::RemoteLink(args) => {
            let title = args.title.as_deref().unwrap_or(&args.url);
            client.add_remote_link(&args.key, &args.url, title).await?
        }
        JiraSubcommand::RemoteLinks(args) => client.get_remote_links(&args.key).await?,
        JiraSubcommand::RemoteLinkDelete(args) => {
            client.delete_remote_link(&args.key, &args.link_id).await?;
            Value::String(format!(
                "Remote link {} deleted from {}",
                args.link_id, args.key
            ))
        }
        JiraSubcommand::Clone(args) => {
            let source = client.get_issue(&args.key, &[]).await?;
            let source_fields = source
                .get("fields")
                .and_then(|f| f.as_object())
                .ok_or_else(|| anyhow::anyhow!("could not read fields from source issue"))?;

            let mut new_fields = serde_json::Map::new();

            for key in [
                "project",
                "issuetype",
                "description",
                "priority",
                "labels",
                "components",
                "fixVersions",
            ] {
                if let Some(val) = source_fields.get(key)
                    && !val.is_null()
                {
                    new_fields.insert(key.to_string(), val.clone());
                }
            }

            let summary = if let Some(s) = &args.summary {
                s.clone()
            } else {
                let original = source_fields
                    .get("summary")
                    .and_then(|s| s.as_str())
                    .unwrap_or("(no summary)");
                format!("[Clone] {original}")
            };
            new_fields.insert("summary".into(), Value::String(summary));

            client
                .create_issue(&json!({ "fields": new_fields }))
                .await?
        }
        JiraSubcommand::Worklog(wl_cmd) => {
            filter::dispatch_worklog(&wl_cmd.command, client).await?
        }
        JiraSubcommand::Filter(f_cmd) => filter::dispatch_filter(&f_cmd.command, client).await?,
        JiraSubcommand::Attach(args) => client.attach_file(&args.key, &args.file).await?,
        JiraSubcommand::Dashboard(cmd) => admin::dispatch_dashboard(&cmd.command, client).await?,
        JiraSubcommand::Field(cmd) => field::dispatch_field(&cmd.command, client).await?,
        JiraSubcommand::User(cmd) => user::dispatch_user(&cmd.command, client).await?,
        JiraSubcommand::Group(cmd) => user::dispatch_group(&cmd.command, client).await?,
        JiraSubcommand::Version(ver_cmd) => {
            admin::dispatch_version(&ver_cmd.command, client).await?
        }
        JiraSubcommand::Component(comp_cmd) => {
            admin::dispatch_component(&comp_cmd.command, client).await?
        }
        JiraSubcommand::Vote(args) => {
            client.vote_issue(&args.key).await?;
            Value::String(format!("Voted for {}", args.key))
        }
        JiraSubcommand::Unvote(args) => {
            client.unvote_issue(&args.key).await?;
            Value::String(format!("Vote removed from {}", args.key))
        }
        JiraSubcommand::Changelog(args) => {
            if args.all {
                let url = format!("{}/issue/{}/changelog", client.base_url(), args.key);
                client
                    .paginate_offset(&url, args.limit, "values", &[])
                    .await?
            } else {
                client
                    .get_changelog(&args.key, args.limit, args.start_at)
                    .await?
            }
        }
        JiraSubcommand::Watch(args) => {
            client.watch_issue(&args.key).await?;
            Value::String(format!("Now watching {}", args.key))
        }
        JiraSubcommand::Unwatch(args) => {
            client.unwatch_issue(&args.key).await?;
            Value::String(format!("Stopped watching {}", args.key))
        }
        JiraSubcommand::Watchers(args) => client.get_watchers(&args.key).await?,
        JiraSubcommand::Notify(args) => {
            let body = read_body_arg(&args.body)?;
            let mut payload = json!({
                "subject": &args.subject,
                "textBody": body,
            });
            if !args.to.is_empty() {
                let users: Vec<Value> = args
                    .to
                    .iter()
                    .map(|id| json!({ "accountId": id }))
                    .collect();
                payload["to"] = json!({ "users": users });
            }
            client.notify_issue(&args.key, &payload).await?;
            Value::String(format!("Notification sent for {}", args.key))
        }
        JiraSubcommand::CreateMeta(args) => {
            client
                .get_create_meta(args.project.as_deref(), args.issue_type.as_deref())
                .await?
        }
        JiraSubcommand::EditMeta(args) => client.get_edit_meta(&args.key).await?,
        JiraSubcommand::IssueType(cmd) => field::dispatch_issue_type(&cmd.command, client).await?,
        JiraSubcommand::Priority(cmd) => field::dispatch_priority(&cmd.command, client).await?,
        JiraSubcommand::Resolution(cmd) => field::dispatch_resolution(&cmd.command, client).await?,
        JiraSubcommand::Status(cmd) => field::dispatch_status(&cmd.command, client).await?,
        JiraSubcommand::Screen(cmd) => workflow::dispatch_screen(&cmd.command, client).await?,
        JiraSubcommand::Workflow(cmd) => workflow::dispatch_workflow(&cmd.command, client).await?,
        JiraSubcommand::WorkflowScheme(cmd) => {
            workflow::dispatch_workflow_scheme(&cmd.command, client).await?
        }
        JiraSubcommand::PermissionScheme(cmd) => {
            workflow::dispatch_permission_scheme(&cmd.command, client).await?
        }
        JiraSubcommand::NotificationScheme(cmd) => {
            workflow::dispatch_notification_scheme(&cmd.command, client).await?
        }
        JiraSubcommand::IssueSecurityScheme(cmd) => {
            workflow::dispatch_issue_security_scheme(&cmd.command, client).await?
        }
        JiraSubcommand::FieldConfig(cmd) => {
            workflow::dispatch_field_config(&cmd.command, client).await?
        }
        JiraSubcommand::ProjectCategory(cmd) => {
            admin::dispatch_project_category(&cmd.command, client).await?
        }
        JiraSubcommand::IssueTypeScheme(cmd) => {
            workflow::dispatch_issue_type_scheme(&cmd.command, client).await?
        }
        JiraSubcommand::Role(cmd) => admin::dispatch_role(&cmd.command, client).await?,
        JiraSubcommand::Banner(cmd) => admin::dispatch_banner(&cmd.command, client).await?,
        JiraSubcommand::Configuration => client.get_configuration().await?,
        JiraSubcommand::Task(cmd) => admin::dispatch_task(&cmd.command, client).await?,
        JiraSubcommand::Attachment(cmd) => {
            admin::dispatch_attachment_admin(&cmd.command, client).await?
        }
        JiraSubcommand::ServerInfo => client.get_server_info().await?,
        JiraSubcommand::Webhook(cmd) => admin::dispatch_webhook(&cmd.command, client).await?,
        JiraSubcommand::AuditRecords(args) => {
            client
                .get_audit_records(
                    args.limit,
                    args.offset,
                    args.filter.as_deref(),
                    args.from.as_deref(),
                    args.to.as_deref(),
                )
                .await?
        }
        JiraSubcommand::Permissions => client.get_all_permissions().await?,
        JiraSubcommand::MyPermissions => client.get_my_permissions().await?,
        JiraSubcommand::Labels(args) => {
            if args.all {
                let url = format!("{}/label", client.base_url());
                client
                    .paginate_offset(&url, args.limit, "values", &[])
                    .await?
            } else {
                client.list_labels(args.limit).await?
            }
        }
    };

    // Start the pager before writing the (potentially long) response so the
    // user can scroll. The pager only engages on Console output to a TTY when
    // the command was a long-form view; everything else stays inline.
    let use_pager = matches!(format, OutputFormat::Console)
        && io.is_stdout_tty()
        && !io.pager_disabled()
        && cmd_uses_pager(cmd);
    if use_pager {
        io.start_pager()?;
    }

    let write_res = write_output(value, format, io, transforms);
    let stop_res = if use_pager { io.stop_pager() } else { Ok(()) };
    write_res?;
    stop_res?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to build a `JiraSearchArgs` with all fields defaulted.
    fn default_search_args() -> JiraSearchArgs {
        JiraSearchArgs {
            jql: None,
            limit: 50,
            all: false,
            fields: "key,summary".to_string(),
            status: None,
            priority: None,
            assignee: None,
            reporter: None,
            r#type: None,
            label: None,
            component: None,
            resolution: None,
            created: None,
            created_after: None,
            updated: None,
            updated_after: None,
            watching: false,
            order_by: None,
            reverse: false,
        }
    }

    #[test]
    fn build_jql_raw_only() {
        let mut args = default_search_args();
        args.jql = Some("project = FOO".to_string());
        let result = build_jql(&args).unwrap();
        assert_eq!(result, "(project = FOO)");
    }

    #[test]
    fn build_jql_raw_parenthesized_with_filter() {
        let mut args = default_search_args();
        args.jql = Some("status = Done OR assignee = me".to_string());
        args.status = Some("Open".to_string());
        let result = build_jql(&args).unwrap();
        assert_eq!(
            result,
            "(status = Done OR assignee = me) AND status = \"Open\""
        );
    }

    #[test]
    fn build_jql_status_only() {
        let mut args = default_search_args();
        args.status = Some("In Progress".to_string());
        let result = build_jql(&args).unwrap();
        assert_eq!(result, "status = \"In Progress\"");
    }

    #[test]
    fn build_jql_empty_returns_error() {
        let args = default_search_args();
        assert!(build_jql(&args).is_err());
    }
}
