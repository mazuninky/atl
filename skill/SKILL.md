---
name: atl
description: >
  Guide for using the `atl` CLI — a unified command-line tool for Atlassian Confluence and Jira.
  Use this skill whenever the user wants to interact with Confluence or Jira from the terminal:
  reading/creating/updating pages, searching issues with JQL or CQL, managing sprints and boards,
  uploading attachments, transitioning issue statuses, working with comments, labels, workflows,
  or any other Atlassian REST API operation. Also use when the user mentions `atl` by name, asks
  about Atlassian CLI tools, wants to script Confluence/Jira automation, or needs help composing
  JQL/CQL queries for use with `atl`. Even if the user just says "create a Jira ticket" or
  "find that Confluence page" without mentioning `atl`, this skill applies when `atl` is installed.
---

# atl CLI

`atl` is a unified, non-interactive CLI for Atlassian **Confluence** and **Jira**. It works with both Cloud and Data Center/Server instances, supports structured output (`console`, `json`, `toon`, `toml`, `csv`), multiple named profiles, and a generic `atl api` passthrough for any REST endpoint.

## Key design principles

- **Non-interactive**: no prompts, no spinners, no colour unless stdout is a TTY. Output is identical under `| cat` and in CI. The only exceptions are `atl init` (setup wizard) and `atl auth login`.
- **Structured output**: every command returns structured data. Use `-F json` to get machine-readable output, `--jq` to filter with jq expressions, `--template` for minijinja templates.
- **Multi-profile**: one config file, many profiles. Switch with `-p <name>` or `ATL_PROFILE` env var.

## Setup

```bash
# Install
curl -sSfL https://raw.githubusercontent.com/mazuninky/atl/master/scripts/install.sh | sh

# Interactive setup wizard (creates config + stores token in OS keyring)
atl init

# Or authenticate manually
atl auth login --domain acme.atlassian.net --email me@acme.com
```

Config lives at `~/.config/atl/atl.toml`. Environment overrides: `ATL_CONFIG`, `ATL_PROFILE`, `ATL_API_TOKEN`.

## Command structure

```
atl [global-flags] <service> <action> [args]
```

### Global flags

| Flag | Short | Purpose |
|---|---|---|
| `--format` | `-F` | Output format: `console` (default), `json`, `toon`, `toml`, `csv` |
| `--profile` | `-p` | Profile name |
| `--jq` | | Filter output with a jq expression |
| `--template` | | Format output with a minijinja template |
| `--no-color` | | Disable colored output |
| `--no-pager` | | Don't pipe through pager |
| `--retries` | | Max HTTP retries on transient errors (default: 3, 0 = off) |
| `--verbose` | `-v` | Increase log verbosity (-v, -vv, -vvv) |
| `--quiet` | `-q` | Suppress all output except errors |
| `--config` | | Path to alternate config file |

### Service aliases

- `confluence` = `conf` = `c`
- `jira` = `j`

## Confluence commands

### Reading and searching

```bash
# Read a page by ID (returns full body)
atl c read 123456
atl c read 123456 --body-format view     # rendered HTML instead of storage format
atl c read 123456 --web                   # open in browser

# Get page metadata (lightweight — no body, includes version number)
# Use `info` instead of `read` when you only need metadata (title, version, status)
atl c info 123456

# Search with CQL
atl c search "space = DEV AND type = page" --limit 10
atl c search "space = DEV AND type = page" --all    # auto-paginate

# Find by title
atl c find --title "Design notes" --space DEV

# List child pages
atl c children 123456
atl c children 123456 --depth 3 --tree    # indented tree view

# List pages in a space (v2 API)
atl c page-list --space-id 65537 --limit 50
```

### Creating and updating

```bash
# Create page from literal body (Confluence storage format)
atl c create --space DEV --title "New page" --body "<p>Hello</p>"

# Create page from markdown file
atl c create --space DEV --title "Design" --body @design.md --input-format markdown

# Create page from stdin
echo "<p>content</p>" | atl c create --space DEV --title "Piped" --body -

# Create with parent
atl c create --space DEV --title "Sub-page" --body @body.md --parent 123456 --input-format markdown

# Update a page (--title, --body, and --version are ALL required)
atl c update 123456 --title "Updated title" --body @new-body.md --version 5 --input-format markdown

# Typical update workflow: get current version first, then update
# atl c info 123456   →  note the version number (e.g. 4)
# atl c update 123456 --title "Same or new title" --body @new.md --version 5

# Update title only
atl c update-title 123456 --title "Better title" --version 5

# Delete (move to trash)
atl c delete 123456
atl c delete 123456 --purge    # permanent delete
```

### Attachments

```bash
atl c attachment list 123456
atl c attachment upload 123456 --file ./diagram.png
atl c attachment download ATT789 --output ./downloads/
atl c attachment delete ATT789
```

### Comments (v2)

```bash
atl c footer-comment list 123456
atl c footer-comment create 123456 --body "Looks good!"
atl c inline-comment list 123456
```

### Other operations

```bash
atl c export 123456 --output-dir ./backup/       # export page + attachments
atl c copy-tree 123456 --target-space NEWSPACE    # copy page tree
atl c versions 123456                              # version history
atl c label list 123456                            # page labels
atl c label add 123456 --labels "review,draft"     # add labels
atl c space list                                   # list spaces
atl c space get DEV                                # space details
```

## Jira commands

### Searching

```bash
# Search with raw JQL
atl j search "project = PROJ AND status = Open" --limit 20
atl j search "assignee = currentUser() AND resolution = Unresolved"

# Search with filter flags (combined via AND)
atl j search --status "In Progress" --assignee currentUser() --type Bug
atl j search --project PROJ --label urgent --order-by priority --reverse

# Fetch all results
atl j search "project = PROJ" --all

# Choose which fields to return
atl j search "project = PROJ" --fields "key,summary,status,created,assignee"
```

### Viewing and creating

```bash
# View a single issue
atl j view PROJ-123
atl j view PROJ-123 --web     # open in browser

# Current user info
atl j me

# Create an issue
atl j create --project PROJ --issue-type Task --summary "Fix login bug"
atl j create --project PROJ --issue-type Bug \
  --summary "Error on save" \
  --description "Steps to reproduce: ..." \
  --priority High \
  --labels "backend,urgent" \
  --component "Auth"

# Create subtask
atl j create --project PROJ --issue-type Sub-task \
  --summary "Write tests" --parent PROJ-123

# With custom fields
atl j create --project PROJ --issue-type Story \
  --summary "Feature X" \
  --custom customfield_10001=team-alpha
```

### Updating and transitioning

```bash
# Update fields
atl j update PROJ-123 --summary "New title" --priority Medium
atl j update PROJ-123 --labels "done,reviewed" --assignee 5b10ac...

# List available transitions
atl j transitions PROJ-123

# Move issue to new status (use transition ID from above)
atl j move PROJ-123 --transition 31

# Assign
atl j assign PROJ-123 5b10ac8d82e05b22cc7d4ef5

# Clone an issue
atl j clone PROJ-123 --summary "Copy of PROJ-123"
```

### Comments

```bash
# Add a comment
atl j comment PROJ-123 "This is done"
atl j comment PROJ-123 @comment.md              # from file
echo "LGTM" | atl j comment PROJ-123 -          # from stdin

# List comments
atl j comments PROJ-123

# Get/delete specific comment
atl j comment-get PROJ-123 10042
atl j comment-delete PROJ-123 10042
```

**Jira Cloud ADF note**: On Jira Cloud, comment `.body` is an Atlassian Document Format (ADF) object, not a plain string. When extracting comment text with `--jq`, you may need a deeper path like `.comments[].body.content[].content[].text` instead of just `.comments[].body`.

### Boards and sprints

```bash
atl j board list
atl j board list --project PROJ
atl j board view 42

atl j sprint list --board-id 42
atl j sprint view 100
atl j sprint issues 100 --limit 50

atl j backlog-move --board-id 42 --issues PROJ-1,PROJ-2
```

### Other Jira operations

```bash
atl j link --link-type Blocks PROJ-1 PROJ-2        # link issues
atl j attach PROJ-123 --file ./screenshot.png       # attach file
atl j watch PROJ-123                                 # watch issue
atl j watchers PROJ-123                              # list watchers
atl j changelog PROJ-123                             # change history
atl j worklog list PROJ-123                          # time tracking
atl j project list                                   # list projects
atl j field list                                     # list fields
atl j filter list                                    # saved filters
```

## Generic REST passthrough (`atl api`)

When a dedicated command doesn't exist, `atl api` gives you authenticated curl over your profile:

```bash
# GET
atl api --service jira rest/api/2/myself
atl api --service confluence /wiki/api/v2/pages --query space-id=123

# With pagination
atl api --service jira rest/api/2/search --query jql='project=TEST' --paginate

# POST with JSON fields
atl api --service jira -X POST rest/api/2/issue \
  --raw-field 'fields={"project":{"key":"TEST"},"summary":"New issue","issuetype":{"name":"Task"}}'

# POST with body from file
atl api --service jira -X POST rest/api/2/issue --input @payload.json

# Preview request without sending
atl api --service jira rest/api/2/myself --preview

# Custom headers
atl api --service jira -H "X-Custom:value" rest/api/2/myself
```

## Output formatting

All commands support the same output pipeline: `-F <format>`, `--jq`, `--template`. Always use the short form `-F` (not `--format`) — it's the idiomatic style.

```bash
# JSON output
atl -F json j search "project = PROJ" --limit 5

# Extract specific fields with jq
atl -F json j view PROJ-123 --jq '.fields.status.name'

# Get just issue keys
atl -F json j search "project = PROJ" --jq '.issues[].key'

# Minijinja template
atl j search "project = PROJ" \
  --template '{% for i in issues %}{{ i.key }}: {{ i.fields.summary }}{% endfor %}'

# CSV for spreadsheet import
atl -F csv j search "project = PROJ" --fields "key,summary,status,priority"

# TOML output
atl -F toml c info 123456
```

## Configuration management

```bash
atl config list                           # list all profiles
atl config show work                      # show profile details
atl config set-default work               # set default profile
atl config set-defaults --project PROJ    # set default Jira project
atl config set-defaults --space DEV       # set default Confluence space
atl config delete old-profile             # delete a profile
```

## Aliases

User-defined command aliases for frequent operations:

```bash
atl alias set mybugs 'jira search "assignee = currentUser() AND type = Bug"'
atl alias list
atl alias delete mybugs

# Use it
atl mybugs
```

## Authentication

```bash
atl auth login                             # interactive wizard
atl auth login --service jira --domain acme.atlassian.net --email me@acme.com
atl auth login --with-token < token.txt    # non-interactive (CI)
atl auth status                            # show auth status
atl auth token --service jira              # print resolved token
atl auth logout                            # remove stored credentials
```

Token resolution order: `ATL_API_TOKEN` env > legacy `api_token` in config TOML > OS keyring.

For CI/scripts, set `ATL_API_TOKEN` environment variable — no keyring needed.

## Body input convention

Commands that accept a body (`--body`, `atl j comment`, `atl api --input`) support three forms:

| Form | Description |
|---|---|
| `"literal string"` | Inline text |
| `@path/to/file` | Read from file |
| `-` | Read from stdin |

## CI / scripting usage

`atl` is non-interactive by design and works well in CI pipelines and shell scripts. Key points:

- **Auth**: set `ATL_API_TOKEN` env var — no keyring or `atl auth login` needed
- **Profile**: set `ATL_PROFILE` or use `-p <name>` if not using default
- **Pager**: disabled automatically when stdout is not a TTY; add `--no-pager` explicitly if piping to a file just to be safe
- **Color**: disabled automatically when stdout is not a TTY; `--no-color` for explicit control
- **Machine output**: always use `-F json` with `--jq` or `--template` for parsing in scripts
- **Pagination**: use `--all` to get complete result sets

```bash
#!/usr/bin/env bash
set -euo pipefail

export ATL_API_TOKEN="${JIRA_TOKEN}"
export ATL_PROFILE="ci"

# Search and extract keys
KEYS=$(atl -F json j search "project = PROJ AND status = Open" --all --jq '.issues[].key')

# Loop over results
for key in $KEYS; do
  echo "Processing: $key"
  atl j comment "$key" "Automated sweep — closing stale issues"
  atl j move "$key" --transition 31
done
```

## Self-update

```bash
atl self check              # check for newer release
atl self update              # download and replace binary
atl self update --to 2026.16.2  # pin to specific version
```

---

## Examples

**Example 1: Find all open bugs assigned to me and export as CSV**
```bash
atl -F csv j search --status Open --type Bug --assignee currentUser() --all \
  --fields "key,summary,priority,created"
```

**Example 2: Create a Confluence page from a markdown file under a parent page**
```bash
atl c create --space DEV --title "Sprint 42 retro" \
  --body @retro.md --input-format markdown --parent 98765
```

**Example 3: Bulk-transition issues from a JQL search**
```bash
# First find the transition ID
atl j transitions PROJ-100

# Then transition each issue
for key in $(atl -F json j search "project = PROJ AND status = 'To Do'" --jq '.issues[].key' --all); do
  atl j move "$key" --transition 31
done
```

**Example 4: Use `atl api` for an endpoint without dedicated command support**
```bash
# Get all dashboards
atl api --service jira rest/api/2/dashboard --paginate

# Create a filter
atl api --service jira -X POST rest/api/2/filter \
  --field name="My filter" --raw-field 'jql="project = PROJ"'
```

**Example 5: Script-friendly: extract issue status into a shell variable**
```bash
STATUS=$(atl -F json j view PROJ-123 --jq '.fields.status.name')
echo "Current status: $STATUS"
```

**Example 6: CI pipeline — create an issue and capture the key**
```bash
export ATL_API_TOKEN="$JIRA_TOKEN"
export ATL_PROFILE="ci"

KEY=$(atl -F json j create --project PROJ --issue-type Bug \
  --summary "Build failure in CI #${BUILD_NUMBER}" \
  --jq '.key')
echo "Created: $KEY"
```

---

## Anti-examples (common mistakes)

**Anti-example 1: Using `--format` for the global flag**
```bash
# WRONG: the long flag is --format but short is -F
atl --format json j view PROJ-123    # works but verbose
atl -F json j view PROJ-123          # preferred, shorter

# WRONG: placing format flag after the subcommand (it's global, works anywhere, but convention is before)
atl j view PROJ-123 -F json          # works but unconventional
```

**Anti-example 2: Forgetting required flags on page update**
```bash
# WRONG: --title, --body, and --version are ALL required for update
atl c update 123456 --body "content"                        # missing --title and --version
atl c update 123456 --body "content" --version 5            # missing --title

# RIGHT: get the current version with `info`, then pass all three required flags
atl c info 123456                     # check current version (e.g. version: 4)
atl c update 123456 --title "Page title" --body @content.md --version 5
```

**Anti-example 3: Using transition name instead of ID**
```bash
# WRONG: move expects a transition ID, not a name
atl j move PROJ-123 --transition "Done"

# RIGHT: get transition IDs first
atl j transitions PROJ-123           # find the ID for "Done"
atl j move PROJ-123 --transition 31  # use the numeric ID
```

**Anti-example 4: Writing to a read-only profile**
```bash
# If the profile has read_only = true, writes will be refused at the client layer.
# The error comes from atl itself, not the server.
atl -p readonly-prod c create --space DEV --title "Test"
# Error: write operations are not allowed for read-only profiles
```

**Anti-example 5: Expecting interactive prompts from commands**
```bash
# WRONG: atl is non-interactive (except init/auth login).
# Don't expect confirmation prompts before delete.
atl c delete 123456   # this will delete immediately, no "are you sure?"
atl j delete PROJ-1   # same — immediate deletion

# Use --preview with atl api if you want to inspect before sending
atl api --service jira -X DELETE rest/api/2/issue/PROJ-1 --preview
```

**Anti-example 6: Trying to use `--body-format markdown` on create (wrong flag name)**
```bash
# WRONG: the flag for input is --input-format, not --body-format
atl c create --space DEV --title "X" --body @doc.md --body-format markdown

# RIGHT:
atl c create --space DEV --title "X" --body @doc.md --input-format markdown

# --body-format is only for reading: it controls whether you get storage or view HTML
atl c read 123456 --body-format view
```

**Anti-example 7: Using `atl c read` when you only need the version number**
```bash
# WRONG: reads the entire page body just to get the version
atl -F json c read 123456 --include-versions --jq '.version.number'

# RIGHT: `info` returns metadata without the body — faster and lighter
atl -F json c info 123456 --jq '.version.number'
```

---

## Quick reference for building `atl` commands

When composing an `atl` command, follow this decision tree:

1. **Which service?** `confluence` (or `c`) / `jira` (or `j`) / `api` (passthrough)
2. **Which action?** Match the verb: `search`, `view`, `create`, `update`, `delete`, `move`, etc.
3. **Output needs?** Add `-F json` for machine parsing, `--jq` for extraction, `-F csv` for spreadsheets
4. **Pagination?** Add `--all` for full result sets, or `--limit N` for bounded queries
5. **Body content?** Use `@file`, `-` (stdin), or inline string
6. **Profile?** Add `-p <name>` or set `ATL_PROFILE` env var if not using default

For endpoints without a dedicated command, fall back to `atl api --service <svc> <endpoint>`.
