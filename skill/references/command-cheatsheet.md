# atl command cheatsheet

Quick reference of all top-level commands and their most common arguments. Read this when you need to find the right command for a task.

## Confluence (`atl confluence` / `atl conf` / `atl c`)

| Command | Purpose | Key flags |
|---|---|---|
| `read <PAGE_ID>` | Read page content | `--body-format`, `--web`, `--include-labels` |
| `info <PAGE_ID>` | Page metadata | |
| `search <CQL>` | CQL search | `--limit`, `--all` |
| `find` | Find by title | `--title`, `--space`, `--limit` |
| `children <PAGE_ID>` | List child pages | `--depth`, `--tree`, `--limit` |
| `page-list` | List pages (v2) | `--space-id`, `--title`, `--status`, `--sort` |
| `create` | Create page | `--space`, `--title`, `--body`, `--parent`, `--input-format` |
| `update <PAGE_ID>` | Update page | `--title`, `--body`, `--version`, `--input-format` |
| `update-title <PAGE_ID>` | Update title only | `--title`, `--version` |
| `delete <PAGE_ID>` | Delete page | `--purge`, `--draft` |
| `export <PAGE_ID>` | Export page + attachments | `--output-dir`, `--body-format` |
| `copy-tree <PAGE_ID>` | Copy page tree | `--target-space`, `--target-parent`, `--depth`, `--dry-run` |
| `attachment list/upload/download/delete` | Manage attachments | `--file`, `--output` |
| `footer-comment list/create/get/delete` | Footer comments (v2) | `--body` |
| `inline-comment list/create/get/delete` | Inline comments (v2) | `--body` |
| `label list/add/remove` | Page labels | `--labels` |
| `property list/get/set/delete` | Content properties | `--key`, `--value` |
| `blog list/get/create/update/delete` | Blog posts | `--space`, `--title`, `--body` |
| `versions <PAGE_ID>` | Version history | `--limit` |
| `version-detail <PAGE_ID> <VERSION>` | Specific version | |
| `space list/get` | Spaces | `--limit` |
| `ancestors <PAGE_ID>` | Page ancestors | |
| `descendants <PAGE_ID>` | Page descendants | `--limit` |
| `likes/likes-count/likes-users` | Page likes | |
| `user` | User operations | |
| `convert-ids` | Convert content IDs | |
| `redact <PAGE_ID>` | Redact content | |

## Jira (`atl jira` / `atl j`)

| Command | Purpose | Key flags |
|---|---|---|
| `search [JQL]` | JQL search | `--limit`, `--all`, `--fields`, `--status`, `--type`, `--assignee`, `--label`, `--order-by` |
| `view <KEY>` | View issue | `--web` |
| `create` | Create issue | `--project`, `--issue-type`, `--summary`, `--description`, `--priority`, `--labels`, `--parent`, `--custom` |
| `update <KEY>` | Update issue | `--summary`, `--description`, `--priority`, `--labels`, `--assignee`, `--custom` |
| `delete <KEY>` | Delete issue | `--delete-subtasks` |
| `move <KEY>` | Transition issue | `--transition` (numeric ID) |
| `assign <KEY> <ACCOUNT_ID>` | Assign issue | |
| `comment <KEY> <BODY>` | Add comment | body supports `@file` / `-` |
| `comments <KEY>` | List comments | |
| `comment-get <KEY> <ID>` | Get comment | |
| `comment-delete <KEY> <ID>` | Delete comment | |
| `transitions <KEY>` | Available transitions | |
| `clone <KEY>` | Clone issue | `--summary` |
| `link` | Link issues | `--link-type`, inward/outward keys |
| `attach <KEY>` | Attach file | `--file` |
| `watch/unwatch <KEY>` | Watch management | |
| `watchers <KEY>` | List watchers | |
| `vote/unvote <KEY>` | Vote management | |
| `changelog <KEY>` | Change history | `--limit`, `--all` |
| `notify <KEY>` | Send notification | `--subject`, `--body`, `--to` |
| `me` | Current user | |
| `board list/view` | Boards | `--project` |
| `sprint list/view/issues` | Sprints | `--board-id` |
| `backlog-move` | Move to backlog | `--board-id`, `--issues` |
| `epic list/view/issues` | Epics | |
| `project list/view` | Projects | |
| `filter list/get/create/update/delete` | Saved filters | |
| `worklog list/add/update/delete` | Time tracking | |
| `field list/get` | Fields | |
| `component list/get/create/update/delete` | Components | |
| `version list/get/create/update/delete` | Versions | |
| `dashboard list/get` | Dashboards | |
| `user search/get` | User operations | |
| `create-meta` | Creation metadata | `--project`, `--issue-type` |
| `edit-meta <KEY>` | Edit metadata | |
| `remote-link/remote-links` | Remote links | |
| `labels` | All labels | `--limit`, `--all` |
| `server-info` | Server info | |
| `permissions/my-permissions` | Permissions | |

## Cross-cutting commands

| Command | Purpose |
|---|---|
| `atl api --service <svc> <endpoint>` | Generic REST passthrough |
| `atl browse <target>` | Open in browser (auto-detects service) |
| `atl config list/show/delete/set-default/set-defaults` | Profile management |
| `atl auth login/logout/status/token` | Authentication |
| `atl alias set/list/delete` | Command aliases |
| `atl init` | Interactive setup wizard |
| `atl self check/update` | Self-update |
| `atl completions <shell>` | Shell completions |
