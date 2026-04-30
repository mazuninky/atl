use serde_json::Value;

use crate::cli::args::*;
use crate::client::JiraClient;

pub(super) async fn dispatch_board(
    cmd: &JiraBoardSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraBoardSubcommand::List(args) => client.get_boards(args.project.as_deref()).await?,
        JiraBoardSubcommand::Get(args) => client.get_board(args.board_id).await?,
        JiraBoardSubcommand::Config(args) => client.get_board_config(args.board_id).await?,
        JiraBoardSubcommand::Issues(args) => {
            let fields: Vec<&str> = args.fields.split(',').map(str::trim).collect();
            if args.all {
                let url = format!("{}/board/{}/issue", client.agile_url(), args.board_id);
                let fields_str = fields.join(",");
                client
                    .paginate_offset(&url, args.limit, "issues", &[("fields", &fields_str)])
                    .await?
            } else {
                client
                    .get_board_issues(args.board_id, args.limit, &fields)
                    .await?
            }
        }
        JiraBoardSubcommand::Backlog(args) => {
            let fields: Vec<&str> = args.fields.split(',').map(str::trim).collect();
            if args.all {
                let url = format!("{}/board/{}/backlog", client.agile_url(), args.board_id);
                let fields_str = fields.join(",");
                client
                    .paginate_offset(&url, args.limit, "issues", &[("fields", &fields_str)])
                    .await?
            } else {
                client
                    .get_board_backlog(args.board_id, args.limit, &fields)
                    .await?
            }
        }
    })
}

#[cfg(test)]
mod tests {
    // All branches are pure HTTP delegation (with a one-liner `split(',').map(trim).collect()`
    // that's not worth its own helper); covered by contract tests in tests/contract_jira_*.rs.
}
