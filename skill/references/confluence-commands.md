# Confluence command reference

Complete list of `atl confluence` (aliases: `conf`, `c`) commands and flags.

## Table of contents

1. [Page operations](#page-operations)
2. [Space operations](#space-operations)
3. [Attachment operations](#attachment-operations)
4. [Comments — footer & inline](#comments)
5. [Label operations](#label-operations)
6. [Property operations](#property-operations)
7. [Blog operations](#blog-operations)
8. [Content types — whiteboard, database, folder](#content-types)
9. [Custom content](#custom-content)
10. [Tasks](#tasks)
11. [Admin — classification, admin-key, user, app-property](#admin)

---

## Page operations

### `atl c read <PAGE_ID>`

Read a page by ID.

| Flag | Short | Default | Description |
|---|---|---|---|
| `--body-format` | | `markdown` | Body format: `markdown` (default), `storage` (raw XHTML), `view` (rendered HTML), `adf` |
| `--no-directives` | | | Strip MyST-style directives (`:::info`, `:status[…]`, etc.) from markdown output |
| `--include-labels` | | | Include labels |
| `--include-properties` | | | Include content properties |
| `--include-operations` | | | Include permitted operations |
| `--include-versions` | | | Include version details |
| `--include-collaborators` | | | Include collaborators |
| `--include-favorited-by` | | | Include favorited-by info |
| `--web` | | | Open in browser instead of printing |

### `atl c info <PAGE_ID>`

Get page metadata (no body — faster than `read`).

### `atl c search <CQL>`

Search pages with CQL.

| Flag | Short | Default | Description |
|---|---|---|---|
| `--limit` | `-l` | 25 | Max results per page |
| `--all` | | | Fetch all results (auto-paginate) |

### `atl c find`

Find pages by title.

| Flag | Short | Required | Default | Description |
|---|---|---|---|---|
| `--title` | `-t` | **yes** | | Page title to search for |
| `--space` | `-s` | | | Space key to search within |
| `--limit` | `-l` | | 25 | Max results |

### `atl c children <PAGE_ID>`

List child pages.

| Flag | Short | Default | Description |
|---|---|---|---|
| `--limit` | `-l` | 25 | Max results |
| `--depth` | `-d` | 1 | Recursion depth (1 = direct children only) |
| `--tree` | | | Display as an indented tree |

### `atl c page-list`

List pages via v2 API.

| Flag | Short | Default | Description |
|---|---|---|---|
| `--space-id` | `-s` | | Space IDs (repeatable) |
| `--title` | `-t` | | Filter by title |
| `--status` | | | Page status |
| `--sort` | | | Sort field |
| `--limit` | `-l` | 25 | Max results |

### `atl c create`

Create a new page.

| Flag | Short | Required | Default | Description |
|---|---|---|---|---|
| `--space` | `-s` | yes* | | Space key (*or `--space-id`) |
| `--space-id` | | yes* | | Space ID (*conflicts with `--space`) |
| `--title` | `-t` | **yes** | | Page title |
| `--body` | `-b` | **yes** | | Body content (`@file`, `-`, or literal) |
| `--parent` | | | | Parent page ID |
| `--input-format` | | | `markdown` | Input format: `markdown` (default), `storage` (raw XHTML), `adf` (raw JSON) |
| `--private` | | | | Create as private (personal) page |
| `--subtype` | | | | Page subtype |
| `--embedded` | | | | Create as embedded content |
| `--root-level` | | | | Create at root level (no parent) |

### `atl c update <PAGE_ID>`

Update an existing page. **All three flags are required.**

| Flag | Short | Required | Default | Description |
|---|---|---|---|---|
| `--title` | `-t` | **yes** | | Page title |
| `--body` | `-b` | **yes** | | Body content (`@file`, `-`, or literal) |
| `--version` | | **yes** | | Version number (current + 1) |
| `--input-format` | | | `markdown` | Input format: `markdown` (default), `storage` (raw XHTML), `adf` (raw JSON) |
| `--version-message` | | | | Version comment |

### `atl c update-title <PAGE_ID>`

Update page title only.

| Flag | Short | Required | Description |
|---|---|---|---|
| `--title` | `-t` | **yes** | New title |
| `--version` | | **yes** | Version number (current + 1) |

### `atl c delete <PAGE_ID>`

| Flag | Description |
|---|---|
| `--purge` | Permanently delete instead of moving to trash |
| `--draft` | Delete draft version only |

### `atl c versions <PAGE_ID>`

Page version history. `--limit -l` (default: 25).

### `atl c version-detail <PAGE_ID> <VERSION>`

Get a specific page version.

### `atl c likes <PAGE_ID>`

Page likes.

### `atl c likes-count <PAGE_ID>`

Like count for a page.

### `atl c likes-users <PAGE_ID>`

Users who liked a page.

### `atl c operations <PAGE_ID>`

Page operations/permissions.

### `atl c ancestors <PAGE_ID>`

Page ancestors.

### `atl c descendants <PAGE_ID>`

Page descendants. `--limit -l` (default: 25).

### `atl c redact <PAGE_ID>`

Redact content from a page.

### `atl c page-custom-content <PAGE_ID>`

List custom content in a page.

| Flag | Short | Required | Default | Description |
|---|---|---|---|---|
| `--content-type` | `-t` | **yes** | | Custom content type |
| `--limit` | `-l` | | 25 | Max results |

### `atl c export <PAGE_ID>`

Export a page with attachments to a local directory.

| Flag | Short | Default | Description |
|---|---|---|---|
| `--output-dir` | `-o` | `.` | Output directory |
| `--body-format` | | `markdown` | Body format: `markdown` (default), `storage`, `view`, `adf` |
| `--no-directives` | | | Strip MyST-style directives from markdown output |

### `atl c copy-tree <SOURCE_PAGE_ID>`

Copy a page tree to another space/parent.

| Flag | Short | Required | Default | Description |
|---|---|---|---|---|
| `--target-space` | | yes* | | Target space key (*or `--target-space-id`) |
| `--target-space-id` | | yes* | | Target space ID |
| `--target-parent` | | | | Target parent page ID |
| `--depth` | `-d` | | 999 | Max depth to copy |
| `--dry-run` | | | | Show what would be copied |
| `--exclude` | | | | Glob pattern to exclude pages by title |

### `atl c convert-ids <IDS...>`

Convert content IDs between formats. Accepts multiple IDs.

---

## Space operations

### `atl c space list`

| Flag | Short | Default | Description |
|---|---|---|---|
| `--limit` | `-l` | 25 | Max results |
| `--all` | | | Fetch all results |

### `atl c space get <SPACE_ID>`

Get space by ID.

### `atl c space create`

| Flag | Short | Required | Description |
|---|---|---|---|
| `--key` | `-k` | **yes** | Space key |
| `--name` | `-n` | **yes** | Space name |
| `--description` | `-d` | | Description |
| `--private` | | | Create as private space |
| `--alias` | | | Space alias |
| `--template-key` | | | Template key for homepage |

### `atl c space delete <SPACE_ID>`

### `atl c space pages <SPACE_ID>`

List pages in space. `--limit -l` (default: 25).

### `atl c space blogposts <SPACE_ID>`

List blog posts in space. `--limit -l` (default: 25).

### `atl c space labels <SPACE_ID>`

List labels in space. `--limit -l` (default: 25).

### `atl c space permissions <SPACE_ID>`

List space permissions. `--limit -l` (default: 25).

### `atl c space permissions-available`

List available space permissions.

### `atl c space content-labels <SPACE_ID>`

List labels of content in space. `--limit -l` (default: 25).

### `atl c space custom-content <SPACE_ID>`

| Flag | Short | Required | Default | Description |
|---|---|---|---|---|
| `--content-type` | `-t` | **yes** | | Custom content type |
| `--limit` | `-l` | | 25 | Max results |

### `atl c space operations <SPACE_ID>`

List permitted operations for a space.

### `atl c space role-assignments <SPACE_ID>`

Get space role assignments. `--limit -l` (default: 25).

### `atl c space set-role-assignments <SPACE_ID>`

| Flag | Short | Required | Description |
|---|---|---|---|
| `--body` | `-b` | **yes** | JSON body with role assignments |

### `atl c space property list/get/set/delete <SPACE_ID>`

Same as page property operations but scoped to a space.

### `atl c space role list/get/create/update/delete <SPACE_ID>`

Space role management:
- `list <SPACE_ID>` — list roles. `--limit -l` (default: 25)
- `get <SPACE_ID> <ROLE_ID>` — get a role
- `create <SPACE_ID> --name -n` — create a role
- `update <SPACE_ID> <ROLE_ID> --name -n` — update a role
- `delete <SPACE_ID> <ROLE_ID>` — delete a role
- `mode <SPACE_ID>` — get space roles mode

---

## Attachment operations

### `atl c attachment list <PAGE_ID>`

| Flag | Short | Default | Description |
|---|---|---|---|
| `--limit` | `-l` | 25 | Max results |
| `--pattern` | | | Glob pattern to filter filenames (e.g. `*.pdf`) |
| `--media-type` | | | Filter by media type (e.g. `image/png`) |
| `--filename` | | | Filter by exact filename |

### `atl c attachment get <ATTACHMENT_ID>`

Get attachment metadata (v2).

### `atl c attachment upload <PAGE_ID>`

| Flag | Short | Required | Description |
|---|---|---|---|
| `--file` | `-f` | **yes** | Path to the file to upload |

### `atl c attachment download <ATTACHMENT_ID>`

| Flag | Short | Required | Description |
|---|---|---|---|
| `--page-id` | | **yes** | Page that owns the attachment |
| `--output` | `-o` | | Output file path |

### `atl c attachment delete <ATTACHMENT_ID>`

### `atl c attachment labels <ATTACHMENT_ID>`

List labels (v2). `--limit -l` (default: 25).

### `atl c attachment comments <ATTACHMENT_ID>`

List comments (v2). `--limit -l` (default: 25).

### `atl c attachment operations <ATTACHMENT_ID>`

Permitted operations (v2).

### `atl c attachment versions <ATTACHMENT_ID>`

Version history (v2). `--limit -l` (default: 25).

### `atl c attachment version-details <ATTACHMENT_ID> <VERSION>`

Get specific version details (v2).

### `atl c attachment property list/get/set/delete`

Property management for attachments (v2). Same pattern as page properties.

---

## Comments

### Footer comments (`atl c footer-comment`)

| Subcommand | Args | Key flags | Description |
|---|---|---|---|
| `list` | `<PAGE_ID>` | `--limit -l` (25) | List footer comments |
| `get` | `<COMMENT_ID>` | | Get a comment |
| `create` | `<PAGE_ID>` | `--body -b` (required) | Create a comment |
| `update` | `<COMMENT_ID>` | `--body -b` (req), `--version` (req) | Update a comment |
| `delete` | `<COMMENT_ID>` | | Delete a comment |
| `children` | `<COMMENT_ID>` | `--limit -l` (25) | Child comments |
| `versions` | `<COMMENT_ID>` | `--limit -l` (25) | Comment versions |
| `likes` | `<COMMENT_ID>` | | Comment likes |
| `operations` | `<COMMENT_ID>` | | Permitted operations |
| `likes-count` | `<COMMENT_ID>` | | Like count |
| `likes-users` | `<COMMENT_ID>` | | Users who liked |
| `version-details` | `<COMMENT_ID> <VER>` | | Specific version |
| `property` | subcommand | | Property management (v2) |

### Inline comments (`atl c inline-comment`)

Same subcommands as footer comments, plus:

| Extra flag | On | Description |
|---|---|---|
| `--resolution-status` | `list` | Filter: `open`, `resolved`, `dangling` |
| `--inline-marker-ref` | `create` | Inline marker reference (required) |
| `--text-selection` | `create` | Text selection to highlight |
| `--resolved` | `update` | Mark as resolved/unresolved |

### Legacy v1 comments (hidden)

These still work but are hidden from help. Prefer v2 commands above.

- `atl c comments <PAGE_ID>` — list (v1)
- `atl c create-comment <PAGE_ID> --body <BODY>` — create (v1)
- `atl c delete-comment <COMMENT_ID>` — delete (v1)

---

## Label operations

| Subcommand | Args | Key flags | Description |
|---|---|---|---|
| `list` | `<PAGE_ID>` | `--prefix` | List labels for a page |
| `add` | `<PAGE_ID> <LABELS...>` | | Add labels (multiple) |
| `remove` | `<PAGE_ID> <LABEL>` | | Remove a label |
| `pages` | `<LABEL_ID>` | `--limit -l` (25) | Pages with this label |
| `blogposts` | `<LABEL_ID>` | `--limit -l` (25) | Blog posts with this label |
| `attachments` | `<LABEL_ID>` | `--limit -l` (25) | Attachments with this label |

---

## Property operations

### Page properties (`atl c property`)

| Subcommand | Args | Description |
|---|---|---|
| `list` | `<PAGE_ID>` | List all properties |
| `get` | `<PAGE_ID> <KEY>` | Get a property by key |
| `set` | `<PAGE_ID> <KEY> --value <JSON>` | Set (create/update) a property |
| `delete` | `<PAGE_ID> <KEY>` | Delete a property |

The same `list/get/set/delete` pattern is available as a nested subcommand on:
`attachment property`, `blog property`, `footer-comment property`, `inline-comment property`, `custom-content property`, `space property`, `whiteboard property`, `database property`, `folder property`.

---

## Blog operations

| Subcommand | Args | Key flags | Description |
|---|---|---|---|
| `list` | | `--space -s`, `--limit -l` (25) | List blog posts |
| `read` | `<BLOG_ID>` | `--body-format` (markdown), `--no-directives`, `--include-*` | Read a blog post |
| `create` | | `--space -s`, `--title -t`, `--body -b`, `--input-format` (markdown) | Create a blog post |
| `update` | `<BLOG_ID>` | `--title -t`, `--body -b`, `--version`, `--input-format` (markdown) | Update a blog post |
| `delete` | `<BLOG_ID>` | `--purge`, `--draft` | Delete a blog post |
| `attachments` | `<BLOG_ID>` | `--limit -l` (25) | List attachments (v2) |
| `labels` | `<BLOG_ID>` | | List labels (v2) |
| `footer-comments` | `<BLOG_ID>` | `--limit -l` (25) | Footer comments (v2) |
| `inline-comments` | `<BLOG_ID>` | `--limit -l` (25) | Inline comments (v2) |
| `versions` | `<BLOG_ID>` | `--limit -l` (25) | Version history (v2) |
| `likes` | `<BLOG_ID>` | | Likes (v2) |
| `operations` | `<BLOG_ID>` | | Permitted operations (v2) |
| `version-details` | `<BLOG_ID> <VER>` | | Specific version (v2) |
| `likes-count` | `<BLOG_ID>` | | Like count (v2) |
| `likes-users` | `<BLOG_ID>` | | Users who liked (v2) |
| `custom-content` | `<BLOG_ID>` | `--content-type -t` (req), `--limit -l` | Custom content (v2) |
| `redact` | `<BLOG_ID>` | | Redact content (v2) |
| `property` | subcommand | | Property management (v2) |

---

## Content types

`whiteboard`, `database`, and `folder` share the same subcommand structure:

| Subcommand | Args | Key flags | Description |
|---|---|---|---|
| `create` | | `--space-id -s` (req), `--title -t`, `--template-key`, `--parent-id` | Create |
| `get` | `<ID>` | | Get by ID |
| `delete` | `<ID>` | | Delete |
| `ancestors` | `<ID>` | | List ancestors |
| `descendants` | `<ID>` | `--limit -l` (25) | List descendants |
| `children` | `<ID>` | `--limit -l` (25) | List direct children |
| `operations` | `<ID>` | | Permitted operations |
| `property` | subcommand | | Property management (v2) |

---

## Custom content

| Subcommand | Args | Key flags | Description |
|---|---|---|---|
| `list` | | `--content-type -t`, `--space-id -s`, `--limit -l` (25) | List custom content |
| `get` | `<ID>` | | Get by ID |
| `create` | | `--content-type -t` (req), `--space-id -s` (req), `--title` (req), `--body -b` (req) | Create |
| `update` | `<ID>` | `--title`, `--body -b`, `--version` (req) | Update |
| `delete` | `<ID>` | | Delete |
| `attachments` | `<ID>` | `--limit -l` (25) | List attachments |
| `children` | `<ID>` | `--limit -l` (25) | List children |
| `labels` | `<ID>` | `--limit -l` (25) | List labels |
| `comments` | `<ID>` | `--limit -l` (25) | List comments |
| `operations` | `<ID>` | | Permitted operations |
| `versions` | `<ID>` | `--limit -l` (25) | Version history |
| `version-details` | `<ID> <VERSION>` | | Specific version |
| `property` | subcommand | | Property management (v2) |

---

## Tasks

| Subcommand | Args | Key flags | Description |
|---|---|---|---|
| `list` | | `--space-id`, `--page-id`, `--status`, `--assignee`, `--limit -l` (25) | List tasks |
| `get` | `<TASK_ID>` | | Get a task |
| `update` | `<TASK_ID>` | `--status` (required) | Update task status |

---

## Admin

### Classification (`atl c classification`)

| Subcommand | Args | Description |
|---|---|---|
| `list` | | List classification levels |
| `get-page` | `<ID>` | Get page classification |
| `set-page` | `<ID> --classification-id` | Set page classification |
| `reset-page` | `<ID>` | Reset page classification |

Same `get-*/set-*/reset-*` pattern for: `blogpost`, `space` (uses `--space-id`), `database`, `whiteboard`.

### Admin key (`atl c admin-key`)

| Subcommand | Description |
|---|---|
| `get` | Get admin key status |
| `enable` | Enable admin key |
| `disable` | Disable admin key |

### User (`atl c user`)

| Subcommand | Args | Description |
|---|---|---|
| `bulk` | `<ACCOUNT_IDS...>` | Bulk lookup users by account IDs |
| `check-access` | `--email` (required) | Check user access |
| `invite` | `--emails` (required, multiple) | Invite users |

### App property (`atl c app-property`)

| Subcommand | Args | Description |
|---|---|---|
| `list` | | List app properties |
| `get` | `<KEY>` | Get a property |
| `set` | `<KEY> --value <JSON>` | Set a property |
| `delete` | `<KEY>` | Delete a property |
