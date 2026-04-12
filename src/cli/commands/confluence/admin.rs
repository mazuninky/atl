use serde_json::Value;

use crate::cli::args::*;
use crate::client::ConfluenceClient;

pub(super) async fn dispatch_task(
    cmd: &ConfluenceTaskSubcommand,
    client: &ConfluenceClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        ConfluenceTaskSubcommand::List(args) => {
            let mut params: Vec<(&str, String)> = vec![("limit", args.limit.to_string())];
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
