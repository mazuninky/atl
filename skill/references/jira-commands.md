# Jira command reference

Complete list of `atl jira` (alias: `j`) commands and flags.

## Table of contents

1. [Issue operations](#issue-operations)
2. [Comments](#comments)
3. [Issue linking](#issue-linking)
4. [Board operations](#board-operations)
5. [Sprint operations](#sprint-operations)
6. [Epic operations](#epic-operations)
7. [Project operations](#project-operations)
8. [Filter operations](#filter-operations)
9. [Worklog operations](#worklog-operations)
10. [Component operations](#component-operations)
11. [Version operations](#version-operations)
12. [Dashboard operations](#dashboard-operations)
13. [User operations](#user-operations)
14. [Group operations](#group-operations)
15. [Field operations](#field-operations)
16. [Admin — types, statuses, schemes, roles, webhooks](#admin)

---

## Issue operations

### `atl j search [JQL]`

Search issues. JQL is optional — filter flags can be used standalone or combined with JQL via AND.

| Flag | Short | Default | Description |
|---|---|---|---|
| `--limit` | `-l` | 50 | Max results per page |
| `--all` | | | Fetch all results (auto-paginate) |
| `--fields` | `-f` | `key,summary,status,assignee,priority` | Fields to return (comma-separated) |
| `--status` | | | Filter by status name |
| `--priority` | | | Filter by priority name |
| `--assignee` | | | Filter by assignee (account ID or `currentUser()`) |
| `--reporter` | | | Filter by reporter |
| `--type` | | | Filter by issue type (Bug, Task, Story, etc.) |
| `--label` | | | Filter by label |
| `--component` | | | Filter by component |
| `--resolution` | | | Filter by resolution |
| `--created` | | | Created on or after date (YYYY-MM-DD) |
| `--created-after` | | | Created after date (YYYY-MM-DD) |
| `--updated` | | | Updated on or after date (YYYY-MM-DD) |
| `--updated-after` | | | Updated after date (YYYY-MM-DD) |
| `--watching` | | | Only issues you are watching |
| `--order-by` | | | Order by field (created, priority, etc.) |
| `--reverse` | | | Reverse sort order |

### `atl j view <KEY>`

View an issue.

| Flag | Description |
|---|---|
| `--web` | Open in browser |

### `atl j create`

Create a new issue.

| Flag | Short | Required | Description |
|---|---|---|---|
| `--project` | | **yes** | Project key |
| `--issue-type` | `-t` | **yes** | Issue type (Task, Bug, Story, etc.) |
| `--summary` | `-s` | **yes** | Summary |
| `--description` | `-d` | | Description |
| `--assignee` | | | Assignee account ID |
| `--priority` | | | Priority name |
| `--labels` | | | Labels (comma-separated) |
| `--parent` | | | Parent issue key (for subtasks) |
| `--fix-version` | | | Fix version(s), comma-separated |
| `--component` | | | Component(s), comma-separated |
| `--custom` | | | Custom field (repeatable): `customfield_10001=value` |

### `atl j update <KEY>`

Update an issue. All flags are optional — only specified fields are changed.

| Flag | Short | Description |
|---|---|---|
| `--summary` | `-s` | New summary |
| `--description` | `-d` | New description |
| `--assignee` | | New assignee account ID |
| `--priority` | | New priority name |
| `--labels` | | Labels (replaces existing, comma-separated) |
| `--fix-version` | | Fix version(s), comma-separated |
| `--component` | | Component(s), comma-separated |
| `--custom` | | Custom field (repeatable) |

### `atl j delete <KEY>`

| Flag | Description |
|---|---|
| `--delete-subtasks` | Also delete subtasks |

### `atl j move <KEY>`

Transition an issue. **Use numeric transition ID, not status name.**

| Flag | Short | Required | Description |
|---|---|---|---|
| `--transition` | `-t` | **yes** | Transition ID (get from `atl j transitions`) |

### `atl j transitions <KEY>`

List available transitions and their IDs.

### `atl j assign <KEY> <ACCOUNT_ID>`

Assign an issue.

### `atl j clone <KEY>`

Clone an issue.

| Flag | Short | Description |
|---|---|---|
| `--summary` | `-s` | Override summary for the clone |

### `atl j attach <KEY>`

Attach a file.

| Flag | Short | Required | Description |
|---|---|---|---|
| `--file` | `-f` | **yes** | Path to file |

### `atl j watch <KEY>`

### `atl j unwatch <KEY>`

### `atl j watchers <KEY>`

### `atl j vote <KEY>`

### `atl j unvote <KEY>`

### `atl j changelog <KEY>`

View issue change history.

| Flag | Short | Default | Description |
|---|---|---|---|
| `--limit` | `-l` | 100 | Max results |
| `--start-at` | | 0 | Start index |
| `--all` | | | Fetch all (auto-paginate) |

### `atl j notify <KEY>`

Send a notification about an issue.

| Flag | Short | Required | Description |
|---|---|---|---|
| `--subject` | `-s` | **yes** | Notification subject |
| `--body` | `-b` | **yes** | Body (`@file`, `-`, or literal) |
| `--to` | | | Recipient account IDs (repeatable) |

### `atl j create-meta`

Get issue creation metadata.

| Flag | Short | Description |
|---|---|---|
| `--project` | | Filter by project key |
| `--issue-type` | `-t` | Filter by issue type name |

### `atl j edit-meta <KEY>`

Get issue edit metadata.

### `atl j me`

Show current user info.

### `atl j configuration`

View system configuration.

### `atl j server-info`

Show server information.

### `atl j permissions`

List all permissions.

### `atl j my-permissions`

List my permissions.

### `atl j labels`

List all labels.

| Flag | Short | Default | Description |
|---|---|---|---|
| `--limit` | `-l` | 1000 | Max results |
| `--all` | | | Fetch all (auto-paginate) |

---

## Comments

### `atl j comment <KEY> <BODY>`

Add a comment. Body supports `@file` and `-` (stdin).

### `atl j comments <KEY>`

List comments for an issue.

### `atl j comment-get <KEY> <COMMENT_ID>`

Get a specific comment.

### `atl j comment-delete <KEY> <COMMENT_ID>`

Delete a comment.

---

## Issue linking

### `atl j link`

Link two issues.

| Arg/Flag | Short | Required | Description |
|---|---|---|---|
| `<INWARD_KEY>` | | **yes** | Inward issue key (positional) |
| `<OUTWARD_KEY>` | | **yes** | Outward issue key (positional) |
| `--link-type` | `-t` | **yes** | Link type name (e.g. "Blocks", "Duplicates") |

### `atl j issue-link-get <ID>`

Get an issue link by ID.

### `atl j issue-link-delete <ID>`

Delete an issue link by ID.

### `atl j link-type`

Issue link type management.

| Subcommand | Args | Key flags | Description |
|---|---|---|---|
| `list` | | | List all link types |
| `get` | `<ID>` | | Get a link type |
| `create` | | `--name -n` (req), `--inward` (req), `--outward` (req) | Create |
| `update` | `<ID>` | `--name -n`, `--inward`, `--outward` | Update |
| `delete` | `<ID>` | | Delete |

### Remote links

| Command | Args | Key flags | Description |
|---|---|---|---|
| `remote-link` | `<KEY> <URL>` | `--title -t` | Add a remote link |
| `remote-links` | `<KEY>` | | List remote links |
| `remote-link-delete` | `<KEY> <LINK_ID>` | | Delete a remote link |

---

## Board operations

### `atl j board list`

| Flag | Description |
|---|---|
| `--project` | Filter by project key |

### `atl j board get <BOARD_ID>`

Get board details.

### `atl j board config <BOARD_ID>`

Get board configuration.

### `atl j board issues <BOARD_ID>`

List all issues on a board.

| Flag | Short | Default | Description |
|---|---|---|---|
| `--limit` | `-l` | 50 | Max results |
| `--all` | | | Fetch all (auto-paginate) |
| `--fields` | `-f` | `key,summary,status,assignee` | Fields to return |

### `atl j board backlog <BOARD_ID>`

List backlog issues for a board. Same flags as `board issues`.

---

## Sprint operations

### `atl j sprint list <BOARD_ID>`

List sprints for a board (board_id is positional).

| Flag | Short | Description |
|---|---|---|
| `--state` | `-s` | Filter: `active`, `closed`, `future` |

### `atl j sprint get <SPRINT_ID>`

Get sprint details.

### `atl j sprint issues <SPRINT_ID>`

List issues in a sprint.

| Flag | Short | Default | Description |
|---|---|---|---|
| `--limit` | `-l` | 50 | Max results |
| `--all` | | | Fetch all (auto-paginate) |
| `--fields` | `-f` | `key,summary,status,assignee` | Fields to return |

### `atl j sprint create`

| Flag | Short | Required | Description |
|---|---|---|---|
| `--board-id` | `-b` | **yes** | Board ID (origin board) |
| `--name` | `-n` | **yes** | Sprint name |
| `--start-date` | | | Start date (ISO 8601) |
| `--end-date` | | | End date (ISO 8601) |
| `--goal` | | | Sprint goal |

### `atl j sprint update <SPRINT_ID>`

| Flag | Short | Description |
|---|---|---|
| `--name` | `-n` | New name |
| `--start-date` | | New start date |
| `--end-date` | | New end date |
| `--goal` | | New goal |
| `--state` | | New state: `active`, `closed`, `future` |

### `atl j sprint delete <SPRINT_ID>`

### `atl j sprint move <SPRINT_ID> <ISSUES...>`

Move issues into a sprint. Issue keys are positional.

---

## Epic operations

### `atl j epic list <BOARD_ID>`

List epics for a board.

### `atl j epic get <EPIC_ID_OR_KEY>`

Get epic details.

### `atl j epic issues <EPIC_ID_OR_KEY>`

List issues in an epic.

| Flag | Short | Default | Description |
|---|---|---|---|
| `--limit` | `-l` | 50 | Max results |
| `--all` | | | Fetch all (auto-paginate) |

### `atl j epic add <EPIC_KEY> <ISSUES...>`

Move issues into an epic.

### `atl j epic remove <ISSUES...>`

Remove issues from their epic.

### `atl j backlog-move <ISSUES...>`

Move issues to backlog. Issue keys are positional.

---

## Project operations

| Subcommand | Args | Key flags | Description |
|---|---|---|---|
| `list` | | | List all projects |
| `get` | `<PROJECT_KEY>` | | Get project details |
| `create` | | `--key -k` (req), `--name -n` (req), `--project-type-key -t` (req), `--lead` (req), `--description -d`, `--template` | Create |
| `update` | `<KEY>` | `--name -n`, `--lead`, `--description -d` | Update |
| `delete` | `<PROJECT_KEY>` | | Delete |
| `statuses` | `<PROJECT_KEY>` | | List statuses |
| `roles` | `<PROJECT_KEY>` | | List roles |
| `archive` | `<PROJECT_KEY>` | | Archive |
| `restore` | `<PROJECT_KEY>` | | Restore |
| `features` | `<PROJECT_KEY>` | | List features |

---

## Filter operations

| Subcommand | Args | Key flags | Description |
|---|---|---|---|
| `list` | | `--name -n`, `--favourites`, `--mine` | List filters |
| `get` | `<ID>` | | Get a filter |
| `create` | | `--name -n` (req), `--jql -j` (req), `--description -d`, `--favourite` | Create |
| `update` | `<ID>` | `--name -n`, `--jql -j`, `--description -d`, `--favourite` | Update |
| `delete` | `<ID>` | | Delete |

---

## Worklog operations

| Subcommand | Args | Key flags | Description |
|---|---|---|---|
| `list` | `<KEY>` | | List worklogs |
| `add` | `<KEY>` | `--time-spent -t` (req), `--comment -c`, `--started` | Add entry |
| `delete` | `<KEY> <WORKLOG_ID>` | | Delete entry |

`--time-spent` accepts formats like `"2h 30m"`, `"1d"`, `"45m"`.
`--started` is ISO 8601 (e.g. `2024-01-15T09:00:00.000+0000`).

---

## Component operations

| Subcommand | Args | Key flags | Description |
|---|---|---|---|
| `list` | `<PROJECT_KEY>` | | List components |
| `get` | `<ID>` | | Get a component |
| `create` | | `--project` (req), `--name -n` (req), `--description -d`, `--lead` | Create |
| `update` | `<ID>` | `--name -n`, `--description -d`, `--lead`, `--assignee-type` | Update |
| `delete` | `<ID>` | | Delete |

---

## Version operations

| Subcommand | Args | Key flags | Description |
|---|---|---|---|
| `list` | `<PROJECT_KEY>` | | List versions |
| `get` | `<ID>` | | Get a version |
| `create` | | `--project` (req), `--name -n` (req), `--description -d`, `--release-date` | Create |
| `update` | `<ID>` | `--name -n`, `--description -d`, `--start-date`, `--release-date`, `--released`, `--archived` | Update |
| `delete` | `<ID>` | | Delete |
| `release` | `<ID>` | `--date` (defaults to today) | Mark as released |

---

## Dashboard operations

| Subcommand | Args | Key flags | Description |
|---|---|---|---|
| `list` | | | List dashboards |
| `get` | `<ID>` | | Get a dashboard |
| `create` | | `--name -n` (req), `--description -d` | Create |
| `update` | `<ID>` | `--name -n`, `--description -d` | Update |
| `delete` | `<ID>` | | Delete |
| `copy` | `<ID>` | `--name -n` | Copy a dashboard |
| `gadgets` | `<ID>` | | List gadgets |
| `add-gadget` | `<DASHBOARD_ID>` | `--uri` (req), `--color`, `--position` (row:col) | Add gadget |
| `update-gadget` | `<DASHBOARD_ID> <GADGET_ID>` | `--color`, `--position` | Update gadget |
| `remove-gadget` | `<DASHBOARD_ID> <GADGET_ID>` | | Remove gadget |

---

## User operations

| Subcommand | Args | Key flags | Description |
|---|---|---|---|
| `search` | `<QUERY>` | `--limit -l` (50) | Search users |
| `get` | `<ACCOUNT_ID>` | | Get user by account ID |
| `list` | | `--limit -l` (50), `--all` | List all users |
| `create` | | `--email -e` (req), `--display-name -n`, `--products` | Create user |
| `delete` | `<ACCOUNT_ID>` | | Delete user |
| `assignable` | `<ISSUE_KEY>` | `--limit -l` (50) | Users assignable to an issue |

---

## Group operations

| Subcommand | Args | Key flags | Description |
|---|---|---|---|
| `list` | | | List groups |
| `get` | `<NAME>` | | Get group details |
| `create` | `<NAME>` | | Create a group |
| `delete` | `<NAME>` | | Delete a group |
| `members` | `<NAME>` | `--limit -l` (50) | List members |
| `add-user` | `<NAME> <ACCOUNT_ID>` | | Add user to group |
| `remove-user` | `<NAME> <ACCOUNT_ID>` | | Remove user from group |
| `search` | `<QUERY>` | `--limit -l` (50) | Search groups |

---

## Field operations

| Subcommand | Args | Key flags | Description |
|---|---|---|---|
| `list` | | `--custom` | List all fields (`--custom` for custom only) |
| `create` | | `--name -n` (req), `--type -t` (req), `--description -d`, `--search-key` | Create custom field |
| `delete` | `<ID>` | | Delete custom field |
| `trash` | `<ID>` | | Move to trash |
| `restore` | `<ID>` | | Restore from trash |

---

## Admin

### Issue type (`atl j issue-type`)

| Subcommand | Args | Key flags | Description |
|---|---|---|---|
| `list` | | | List all issue types |
| `get` | `<ID>` | | Get by ID |
| `create` | | `--name -n` (req), `--description -d`, `--type -t` (default: standard) | Create |
| `update` | `<ID>` | `--name -n`, `--description -d` | Update |
| `delete` | `<ID>` | | Delete |

### Priority (`atl j priority`)

| Subcommand | Args | Key flags | Description |
|---|---|---|---|
| `list` | | | List all |
| `get` | `<ID>` | | Get by ID |
| `create` | | `--name -n` (req), `--description -d`, `--status-color` (#ffffff) | Create |
| `update` | `<ID>` | `--name -n`, `--description -d`, `--status-color` | Update |
| `delete` | `<ID>` | | Delete |

### Resolution (`atl j resolution`)

CRUD: `list`, `get <ID>`, `create --name -n --description -d`, `update <ID>`, `delete <ID>`.

### Status (`atl j status`)

- `list` — list all statuses
- `get <ID>` — get by ID
- `categories` — list status categories

### Screen (`atl j screen`)

| Subcommand | Args | Key flags | Description |
|---|---|---|---|
| `list` | | | List screens |
| `get` | `<ID>` | | Get by ID |
| `create` | | `--name -n` (req), `--description -d` | Create |
| `delete` | `<ID>` | | Delete |
| `tabs` | `<ID>` | | List tabs |
| `fields` | `<SCREEN_ID> <TAB_ID>` | | List fields in a tab |

### Workflow (`atl j workflow`)

- `list` — list all workflows
- `get <ID>` — get by ID

### Scheme management

The following use the same CRUD pattern: `list`, `get <ID>`, `create --name -n --description -d`, `update <ID> --name -n --description -d`, `delete <ID>`:

- `atl j workflow-scheme`
- `atl j permission-scheme`
- `atl j notification-scheme`
- `atl j issue-security-scheme`
- `atl j issue-type-scheme` (also has `--default-issue-type-id` on create)

### Field config (`atl j field-config`)

- `list` — list all
- `get <ID>` — get by ID
- `create --name -n --description -d` — create
- `delete <ID>` — delete

### Role (`atl j role`)

| Subcommand | Args | Key flags | Description |
|---|---|---|---|
| `list` | | | List all roles |
| `get` | `<ID>` | | Get by ID |
| `create` | | `--name -n` (req), `--description -d` | Create |
| `delete` | `<ID>` | | Delete |

### Banner (`atl j banner`)

| Subcommand | Args | Key flags | Description |
|---|---|---|---|
| `get` | | | Get announcement banner |
| `set` | | `--message -m` (req, HTML), `--is-enabled`, `--visibility` (public/private) | Set banner |

### Async task (`atl j task`)

- `get <ID>` — get task status
- `cancel <ID>` — cancel task

### Attachment admin (`atl j attachment`)

- `get <ID>` — get attachment by ID
- `delete <ID>` — delete attachment
- `meta` — get upload metadata/settings

### Project category (`atl j project-category`)

CRUD: `list`, `get <ID>`, `create --name -n --description -d`, `update <ID>`, `delete <ID>`.

### Webhook (`atl j webhook`)

| Subcommand | Args | Key flags | Description |
|---|---|---|---|
| `list` | | | List webhooks |
| `get` | `<ID>` | | Get by ID |
| `create` | | `--name -n` (req), `--url -u` (req), `--events -e` (req, comma-sep), `--jql` | Create |
| `delete` | `<ID>` | | Delete |

### Audit records (`atl j audit-records`)

| Flag | Short | Default | Description |
|---|---|---|---|
| `--limit` | `-l` | 100 | Max results |
| `--offset` | | 0 | Offset |
| `--filter` | `-f` | | Filter text |
| `--from` | | | From date (ISO 8601) |
| `--to` | | | To date (ISO 8601) |
