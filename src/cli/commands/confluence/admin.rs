use serde_json::Value;

use crate::cli::args::*;
use crate::client::ConfluenceClient;

/// Build the list of `(key, value)` query parameters for `tasks list`.
/// `limit` is always included; the four optional filters
/// (`space-id`, `page-id`, `status`, `assignee`) are appended only when set,
/// preserving that source order so logs and HTTP query strings are
/// deterministic.
pub(super) fn build_task_list_params(args: &ConfluenceTaskListArgs) -> Vec<(&'static str, String)> {
    let mut params: Vec<(&'static str, String)> = vec![("limit", args.limit.to_string())];
    if let Some(s) = &args.space_id {
        params.push(("space-id", s.clone()));
    }
    if let Some(p) = &args.page_id {
        params.push(("page-id", p.clone()));
    }
    if let Some(st) = &args.status {
        params.push(("status", st.clone()));
    }
    if let Some(a) = &args.assignee {
        params.push(("assignee", a.clone()));
    }
    params
}

pub(super) async fn dispatch_task(
    cmd: &ConfluenceTaskSubcommand,
    client: &ConfluenceClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        ConfluenceTaskSubcommand::List(args) => {
            let params = build_task_list_params(args);
            let param_refs: Vec<(&str, &str)> =
                params.iter().map(|(k, v)| (*k, v.as_str())).collect();
            client.list_tasks_v2(&param_refs).await?
        }
        ConfluenceTaskSubcommand::Get(args) => client.get_task_v2(&args.task_id).await?,
        ConfluenceTaskSubcommand::Update(args) => {
            client.update_task_v2(&args.task_id, &args.status).await?
        }
    })
}

pub(super) async fn dispatch_classification(
    cmd: &ConfluenceClassificationSubcommand,
    client: &ConfluenceClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        ConfluenceClassificationSubcommand::List => client.list_classification_levels_v2().await?,
        ConfluenceClassificationSubcommand::GetPage(args) => {
            client
                .get_content_classification_v2("pages", &args.id)
                .await?
        }
        ConfluenceClassificationSubcommand::SetPage(args) => {
            client
                .set_content_classification_v2("pages", &args.id, &args.classification_id)
                .await?
        }
        ConfluenceClassificationSubcommand::ResetPage(args) => {
            client
                .reset_content_classification_v2("pages", &args.id)
                .await?;
            Value::String("Classification reset".into())
        }
        ConfluenceClassificationSubcommand::GetBlogpost(args) => {
            client
                .get_content_classification_v2("blogposts", &args.id)
                .await?
        }
        ConfluenceClassificationSubcommand::SetBlogpost(args) => {
            client
                .set_content_classification_v2("blogposts", &args.id, &args.classification_id)
                .await?
        }
        ConfluenceClassificationSubcommand::ResetBlogpost(args) => {
            client
                .reset_content_classification_v2("blogposts", &args.id)
                .await?;
            Value::String("Classification reset".into())
        }
        ConfluenceClassificationSubcommand::GetSpace(args) => {
            client
                .get_content_classification_v2("spaces", &args.space_id)
                .await?
        }
        ConfluenceClassificationSubcommand::SetSpace(args) => {
            client
                .set_content_classification_v2("spaces", &args.id, &args.classification_id)
                .await?
        }
        ConfluenceClassificationSubcommand::ResetSpace(args) => {
            client
                .reset_content_classification_v2("spaces", &args.space_id)
                .await?;
            Value::String("Classification reset".into())
        }
        ConfluenceClassificationSubcommand::GetDatabase(args) => {
            client
                .get_content_classification_v2("databases", &args.id)
                .await?
        }
        ConfluenceClassificationSubcommand::SetDatabase(args) => {
            client
                .set_content_classification_v2("databases", &args.id, &args.classification_id)
                .await?
        }
        ConfluenceClassificationSubcommand::ResetDatabase(args) => {
            client
                .reset_content_classification_v2("databases", &args.id)
                .await?;
            Value::String("Classification reset".into())
        }
        ConfluenceClassificationSubcommand::GetWhiteboard(args) => {
            client
                .get_content_classification_v2("whiteboards", &args.id)
                .await?
        }
        ConfluenceClassificationSubcommand::SetWhiteboard(args) => {
            client
                .set_content_classification_v2("whiteboards", &args.id, &args.classification_id)
                .await?
        }
        ConfluenceClassificationSubcommand::ResetWhiteboard(args) => {
            client
                .reset_content_classification_v2("whiteboards", &args.id)
                .await?;
            Value::String("Classification reset".into())
        }
    })
}

#[cfg(test)]
mod tests {
    // `dispatch_classification` is pure HTTP routing on the resource type
    // string — covered by contract tests in tests/contract_confluence_v*.rs.
    // Only `build_task_list_params` has local payload-shaping logic and is
    // unit-tested here.

    use super::*;

    fn task_list_args(
        limit: u32,
        space_id: Option<&str>,
        page_id: Option<&str>,
        status: Option<&str>,
        assignee: Option<&str>,
    ) -> ConfluenceTaskListArgs {
        ConfluenceTaskListArgs {
            space_id: space_id.map(String::from),
            page_id: page_id.map(String::from),
            status: status.map(String::from),
            assignee: assignee.map(String::from),
            limit,
        }
    }

    #[test]
    fn task_params_contain_only_limit_when_no_filters() {
        let params = build_task_list_params(&task_list_args(25, None, None, None, None));
        assert_eq!(params, vec![("limit", "25".to_string())]);
    }

    #[test]
    fn task_params_emit_limit_in_first_position() {
        // Atlassian's API treats the `limit` filter as a hint; when ordering
        // matters for query-string canonicalization we want it first.
        let params = build_task_list_params(&task_list_args(50, Some("S"), None, None, None));
        assert_eq!(params[0].0, "limit", "first param must be `limit`");
        assert_eq!(params[0].1, "50");
    }

    #[test]
    fn task_params_include_all_four_filters_in_source_order() {
        let params = build_task_list_params(&task_list_args(
            10,
            Some("100"),
            Some("200"),
            Some("incomplete"),
            Some("acc-1"),
        ));
        assert_eq!(
            params,
            vec![
                ("limit", "10".to_string()),
                ("space-id", "100".to_string()),
                ("page-id", "200".to_string()),
                ("status", "incomplete".to_string()),
                ("assignee", "acc-1".to_string()),
            ]
        );
    }

    #[test]
    fn task_params_skip_unset_filters_preserving_order() {
        // Setting space-id and assignee but not page-id/status — order of the
        // set ones should still match the source order in the helper.
        let params =
            build_task_list_params(&task_list_args(10, Some("100"), None, None, Some("acc-1")));
        assert_eq!(
            params,
            vec![
                ("limit", "10".to_string()),
                ("space-id", "100".to_string()),
                ("assignee", "acc-1".to_string()),
            ]
        );
    }

    #[test]
    fn task_params_use_kebab_case_keys() {
        // The Confluence API uses kebab-case in query string keys
        // (`space-id`, `page-id`), not the snake_case Rust identifiers.
        let params = build_task_list_params(&task_list_args(5, Some("S"), Some("P"), None, None));
        let keys: Vec<&str> = params.iter().map(|(k, _)| *k).collect();
        assert!(keys.contains(&"space-id"), "expected `space-id` key");
        assert!(keys.contains(&"page-id"), "expected `page-id` key");
        assert!(!keys.contains(&"space_id"), "snake_case must not leak");
        assert!(!keys.contains(&"page_id"), "snake_case must not leak");
    }

    #[test]
    fn task_params_status_filter_passed_through_verbatim() {
        // We do not validate status here — the server is the authority.
        let params =
            build_task_list_params(&task_list_args(5, None, None, Some("anything-goes"), None));
        let status = params
            .iter()
            .find(|(k, _)| *k == "status")
            .map(|(_, v)| v.as_str());
        assert_eq!(status, Some("anything-goes"));
    }

    #[test]
    fn task_params_zero_limit_emitted_as_string() {
        let params = build_task_list_params(&task_list_args(0, None, None, None, None));
        assert_eq!(params, vec![("limit", "0".to_string())]);
    }
}
