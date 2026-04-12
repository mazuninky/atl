# E2E Contract Tests with Prism

## Context

atl makes HTTP calls to 3 Atlassian APIs:
- **Jira Platform REST API** (`/rest/api/2/`) — 162 operations across 96 paths
- **Jira Software/Agile API** (`/rest/agile/1.0/`) — 22 operations across 14 paths
- **Confluence v1 REST API** (`/wiki/rest/api/`) — 6 methods still on v1 (search, upload/download attachment, labels add/remove)
- **Confluence v2 REST API** (`/wiki/api/v2/`) — ~200 operations across 146 paths (includes 20+ methods migrated from v1)

Goal: validate that atl's HTTP requests conform to Atlassian's official OpenAPI specs using [Prism](https://github.com/stoplightio/prism) as a mock server. Prism validates request structure (path, method, headers, query params, body schema) against the spec and returns realistic mock responses.

**Total: ~392 operations → ~784+ test cases (positive + negative)**

### Confluence v1 → v2 Migration Status

The v1-to-v2 migration is **largely complete** (commit `8e443e3`). Most client methods now use the v2 API (`/wiki/api/v2/`).

**Still on v1** (6 methods — these paths exist in the v1 spec):
| Method | v1 Endpoint |
|--------|-------------|
| `search` / `search_all` | `GET /content/search` |
| `download_attachment` | `GET /content/{id}/download` |
| `upload_attachment` | `POST /content/{id}/child/attachment` |
| `add_labels` | `POST /content/{id}/label` |
| `remove_label` | `DELETE /content/{id}/label/{label}` |

**Migrated to v2** (20+ methods):
| Method | v2 Endpoint |
|--------|-------------|
| `get_page` / `get_page_info` | `GET /pages/{id}` |
| `create_page` | `POST /pages` |
| `update_page` | `PUT /pages/{id}` |
| `delete_page` | `DELETE /pages/{id}` |
| `get_children` / `get_children_recursive` | `GET /pages/{id}/children` |
| `get_spaces` | `GET /spaces` |
| `get_attachments` | `GET /pages/{id}/attachments` |
| `delete_attachment` | `DELETE /attachments/{id}` |
| `get_comments` | `GET /pages/{id}/footer-comments` |
| `create_comment` | `POST /footer-comments` |
| `delete_comment` | `DELETE /footer-comments/{id}` |
| `get_properties` / `get_property` | `GET /pages/{id}/properties` |
| `set_property` | `POST/PUT /pages/{id}/properties[/{id}]` |
| `delete_property` | `DELETE /pages/{id}/properties/{id}` |
| `get_labels` | `GET /pages/{id}/labels` |
| `list_blog_posts` | `GET /blogposts` |
| `get_blog_post` | `GET /blogposts/{id}` |
| `create_blog_post` | `POST /blogposts` |
| `update_blog_post` | `PUT /blogposts/{id}` |
| `delete_blog_post` | `DELETE /blogposts/{id}` |

**Test strategy**: v1 tests cover only the 6 remaining v1 methods. All migrated methods are tested via v2 spec/Prism instance.

---

## 1. OpenAPI Specs

### Sources

| Spec | URL | Paths | Size |
|------|-----|-------|------|
| Jira Platform | `https://developer.atlassian.com/cloud/jira/platform/swagger-v3.v3.json` | 421 | 2.4MB |
| Jira Software (Agile) | `https://developer.atlassian.com/cloud/jira/software/swagger.v3.json` | 68 | 581KB |
| Confluence v1 | `https://developer.atlassian.com/cloud/confluence/swagger.v3.json` | 89 | 413KB |
| Confluence v2 | `https://dac-static.atlassian.com/cloud/confluence/rest/v2/swagger.v3.json` (extracted from HTML) | 146 | ~846KB |

### Local storage

```text
tests/contract/specs/
├── jira-platform.v3.json
├── jira-software.v3.json
├── confluence.v3.json          # v1 API
└── confluence-v2.v3.json       # v2 API
```

### Spec Patching

`tests/contract/patch_specs.py` runs over **every** spec and produces a
`*.patched.json` sibling that the contract tests consume. The patcher
applies the following transforms:

- **Rename** `/rest/api/3/` → `/rest/api/2/` in `jira-platform` so the
  spec paths match the URLs atl actually sends.
- **Parse stringified JSON examples.** Atlassian ships ~770 examples as
  JSON-encoded strings (`"example": "{\"foo\": 1}"`). Prism returns them
  verbatim, so without patching every response body is a quoted string
  instead of an object.
- **Rewrite query parameters declared as `array of object`** to plain
  `string` (Jira Agile only). Such schemas are nonsensical for query
  strings and Prism rejects them outright.
- **Rewrite multipart bodies declared as `array of MultipartFile`** (a
  Spring Java DTO dump in the Jira spec) to a real `multipart/form-data`
  schema with a binary `file` field.
- **Strip per-operation `security` requirements** so Prism validates the
  request shape only. Endpoints like `/app/properties` require OAuth
  scopes; atl always uses Basic auth, so Prism would otherwise return
  401 even when the request is well-formed.
- **Prefix every path with the server-URL basepath** (Confluence v2
  only). Prism does **not** honour the basepath component of
  `servers[].url`, so spec paths declared as `/pages/{id}` would be
  served at `/pages/{id}` instead of `/wiki/api/v2/pages/{id}`. The
  patcher moves the basepath into the path keys and clears the server
  URL.

After patching the test files load the `.patched.json` artefacts (e.g.
`tests/contract/specs/confluence-v2.patched.json`).

**3 paths missing from Jira spec** (atl uses them but spec doesn't
define them):

- `/rest/api/2/workflow/{id}` — `jira workflow get`
- `/rest/api/2/issuetypescheme/{id}` — `jira issue-type-scheme get/update/delete`
- `/rest/api/2/webhook/{webhookId}` — `jira webhook get/delete`

These will be skipped in contract tests until Atlassian adds them to the spec.

---

## 2. Test Infrastructure

### File structure

```text
tests/
├── contract/
│   ├── specs/                    # OpenAPI specs (downloaded, gitignored)
│   ├── patch_specs.py            # Spec patching: /rest/api/3/ → /rest/api/2/
│   └── download_specs.sh         # Idempotent spec download from Atlassian
├── common/
│   ├── mod.rs                    # Re-exports
│   ├── prism.rs                  # PrismServer: start/stop/health-check
│   ├── atl.rs                   # AtlRunner: build binary, run commands, assert
│   └── config.rs                 # Generate temp atl.toml pointing to Prism
├── contract_jira_platform.rs     # Jira REST API v2 tests
├── contract_jira_agile.rs        # Jira Agile API tests
├── contract_confluence_v1.rs     # Confluence v1 tests (spec-covered paths only)
├── contract_confluence_v2.rs     # Confluence v2 tests
└── contract_cross_cutting.rs     # Auth, config, output format tests
```

### PrismServer (`tests/common/prism.rs`)

```rust
pub struct PrismServer {
    process: Child,
    port: u16,
    base_url: String,
    /// Background reader thread captures Prism stderr so a crashed child
    /// surfaces with diagnostic context instead of a generic timeout.
    stderr_buf: Arc<Mutex<Vec<u8>>>,
}

impl PrismServer {
    /// Spawn Prism in **static** mock mode (`--dynamic` is avoided —
    /// json-schema-faker crashes on the deeply recursive Atlassian
    /// schemas). Picks a random free port and returns once Prism is
    /// accepting TCP connections.
    pub fn start(spec_path: &str) -> Self { ... }

    /// Poll TCP readiness (up to 90 seconds total) and call `try_wait()`
    /// every iteration; if the child exits before becoming ready,
    /// panic immediately with the captured stderr instead of waiting
    /// for the full timeout.
    fn wait_ready(&mut self) { ... }

    pub fn base_url(&self) -> &str { ... }
    pub fn port(&self) -> u16 { ... }
}

impl Drop for PrismServer {
    fn drop(&mut self) {
        self.process.kill().ok();
        self.process.wait().ok();
    }
}
```

One PrismServer per spec per test file. Use `std::sync::LazyLock` for shared instances:

```rust
static JIRA_PLATFORM: LazyLock<PrismServer> = LazyLock::new(|| {
    PrismServer::start("tests/contract/specs/jira-platform.patched.json")
});
```

### AtlRunner (`tests/common/atl.rs`)

```rust
pub struct AtlRunner {
    config_path: PathBuf,     // temp config file
}

impl AtlRunner {
    /// Construct a runner bound to a config file. The binary is resolved
    /// via `assert_cmd::cargo::cargo_bin("atl")` on every `run` call.
    pub fn new(config_path: &Path) -> Self { ... }

    /// Run atl with given args, return (exit_code, stdout, stderr)
    pub fn run(&self, args: &[&str]) -> AtlResult { ... }

    /// Assert success (exit 0) and return stdout
    pub fn run_ok(&self, args: &[&str]) -> String { ... }

    /// Assert failure with specific exit code
    pub fn run_err(&self, args: &[&str], expected_code: i32) -> String { ... }
}
```

### Config generator (`tests/common/config.rs`)

Generates temp `atl.toml` that points to Prism:

```toml
default_profile = "test"

[profiles.test]
default_project = "TEST"
default_space = "TEST"

[profiles.test.jira]
domain = "http://localhost:{jira_port}"
email = "test@example.com"
api_token = "test-token"
auth_type = "basic"

[profiles.test.confluence]
domain = "http://localhost:{confluence_port}"
email = "test@example.com"
api_token = "test-token"
auth_type = "basic"
api_path = "/wiki/rest/api"
```

> **Note on Confluence v1 vs v2 Prism instances.** `ConfluenceClient`
> derives `base_url_v2` from `api_path` by replacing `/rest/api` → `/api/v2`,
> so both API families resolve to the **same domain/port**. A single test
> binary cannot serve v1 and v2 from one Prism — they would collide on the
> same host. The actual layout in this repo is:
>
> - `tests/contract_confluence_v1.rs` configures `domain` to a Prism running
>   the v1 spec only and exercises the 6 client methods that still hit v1
>   paths.
> - `tests/contract_confluence_v2.rs` configures `domain` to a Prism running
>   the v2 spec only and exercises every client method that uses
>   `base_url_v2`.
> - `tests/contract_cross_cutting.rs` runs both Prism instances side by side
>   and uses a single shared `TestConfig` because cross-cutting tests do not
>   need both API families to resolve through the same domain.
>
> **Reverse-proxy alternative.** A single reverse proxy (caddy/nginx) that
> dispatches by URL prefix would let `domain` point at one shared port and
> route `/wiki/rest/api/*` and `/wiki/api/v2/*` to the appropriate Prism
> backend. We do not use this layout because per-binary isolation is
> simpler, but it remains a valid option for environments where the test
> layout must mimic a real Confluence host.

---

## 3. Test Strategies

### Positive (happy path)
- Prism in **mock mode** (default) — returns 2xx responses generated from spec examples/schemas
- atl command exits 0
- stdout contains expected output (parseable JSON/table)
- For `--format json`: validate output is valid JSON

### Negative (error handling)
1. **Invalid arguments**: wrong arg combinations, missing required args → clap error, exit 2
2. **Not found (404)**: Prism returns 404 for unknown IDs → atl exit 2 (`NOT_FOUND`)
3. **Auth error (401/403)**: Separate Prism instance or `Prefer: code=401` → atl exit 4 (`AUTH_ERROR`)
4. **Server error (500)**: `Prefer: code=500` → atl exit 1 (`RUNTIME_ERROR`)
5. **Read-only mode**: config with `read_only = true` → write commands fail before HTTP call, exit 3 (`CONFIG_ERROR`)

---

## 4. Endpoint Test Matrix

### 4.1 Jira Platform (`contract_jira_platform.rs`)

**162 operations across 96 paths. ~324 test cases.**

#### Issues — Core CRUD (32 tests)

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| GET /search | `jira search "project=TEST"` | exit 0, JSON output | invalid JQL → error |
| GET /search (structured) | `jira search --status Open --assignee currentUser()` | exit 0 | no results → empty |
| POST /issue | `jira create -p TEST -t Task -s "Title"` | exit 0, key returned | missing required fields |
| GET /issue/{key} | `jira view TEST-1` | exit 0, issue data | nonexistent key → 404 |
| PUT /issue/{key} | `jira update TEST-1 --summary "New"` | exit 0 | read-only mode → fail |
| DELETE /issue/{key} | `jira delete TEST-1` | exit 0 | read-only → fail |
| DELETE /issue (subtasks) | `jira delete TEST-1 --delete-subtasks` | exit 0 | — |
| POST /issue/{key}/transitions | `jira move TEST-1 -t 31` | exit 0 | invalid transition |
| GET /issue/{key}/transitions | `jira transitions TEST-1` | exit 0 | — |
| PUT /issue/{key}/assignee | `jira assign TEST-1 account123` | exit 0 | read-only |
| POST /issue/{key}/comment | `jira comment TEST-1 "text"` | exit 0 | empty body |
| GET /issue/{key}/comment | `jira comments TEST-1` | exit 0 | — |
| GET /issue/{key}/comment/{id} | `jira comment-get TEST-1 10001` | exit 0 | 404 |
| DELETE /issue/{key}/comment/{id} | `jira comment-delete TEST-1 10001` | exit 0 | read-only |
| GET /issue/createmeta | `jira create-meta` | exit 0 | — |
| GET /issue/{key}/editmeta | `jira edit-meta TEST-1` | exit 0 | — |

#### Issues — Worklog (6 tests)

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| GET worklog | `jira worklog list TEST-1` | exit 0 | — |
| POST worklog | `jira worklog add TEST-1 --time-spent 2h` | exit 0 | missing time |
| DELETE worklog | `jira worklog delete TEST-1 10001` | exit 0 | read-only |

#### Issues — Watchers (6 tests)

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| GET watchers | `jira watchers TEST-1` | exit 0 | — |
| POST watch | `jira watch TEST-1` | exit 0 | — |
| DELETE unwatch | `jira unwatch TEST-1` | exit 0 | — |

#### Issues — Vote (4 tests)

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| POST vote | `jira vote TEST-1` | exit 0 | — |
| DELETE unvote | `jira unvote TEST-1` | exit 0 | — |

#### Issues — Changelog (4 tests)

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| GET changelog | `jira changelog TEST-1` | exit 0 | — |
| GET changelog (paginated) | `jira changelog TEST-1 --limit 10 --start-at 5` | exit 0 | — |

#### Issues — Attachments (4 tests)

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| POST attachment | `jira attach TEST-1 -f /tmp/test.txt` | exit 0 | missing file |

#### Issues — Links (16 tests)

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| POST issueLink | `jira link -t Blocks TEST-1 TEST-2` | exit 0 | invalid type |
| GET issueLink/{id} | `jira issue-link-get 10001` | exit 0 | 404 |
| DELETE issueLink/{id} | `jira issue-link-delete 10001` | exit 0 | read-only |
| GET issueLinkType | `jira link-type list` | exit 0 | — |
| GET issueLinkType/{id} | `jira link-type get 10001` | exit 0 | 404 |
| POST issueLinkType | `jira link-type create --name T --inward I --outward O` | exit 0 | — |
| PUT issueLinkType/{id} | `jira link-type update 10001 --name X` | exit 0 | — |
| DELETE issueLinkType/{id} | `jira link-type delete 10001` | exit 0 | — |

#### Issues — Remote Links (6 tests)

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| POST remotelink | `jira remote-link TEST-1 https://x.com -t "Doc"` | exit 0 | missing URL |
| GET remotelinks | `jira remote-links TEST-1` | exit 0 | — |
| DELETE remotelink/{id} | `jira remote-link-delete TEST-1 10001` | exit 0 | read-only |

#### Issues — Notify (4 tests)

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| POST notify | `jira notify TEST-1 -s "Subj" -b "Body"` | exit 0 | missing subject |

#### Clone (4 tests)

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| clone (composite) | `jira clone TEST-1` | exit 0 | 404 |

#### Projects (20 tests)

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| GET /project | `jira project list` | exit 0 | — |
| GET /project/{key} | `jira project get TEST` | exit 0 | 404 |
| POST /project | `jira project create --key T --name "Test" --project-type-key software --lead acc1` | exit 0 | missing fields |
| PUT /project/{key} | `jira project update TEST --name "New"` | exit 0 | read-only |
| DELETE /project/{key} | `jira project delete TEST` | exit 0 | read-only |
| GET statuses | `jira project statuses TEST` | exit 0 | — |
| GET roles | `jira project roles TEST` | exit 0 | — |
| POST archive | `jira project archive TEST` | exit 0 | — |
| POST restore | `jira project restore TEST` | exit 0 | — |
| GET features | `jira project features TEST` | exit 0 | — |

#### User & Auth (14 tests)

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| GET /myself | `jira me` | exit 0 | — |
| GET /user | `jira user get acc123` | exit 0 | 404 |
| GET /user/search | `jira user search "john"` | exit 0 | — |
| GET /users/search | `jira user list` | exit 0 | — |
| GET /user/assignable/search | `jira user assignable TEST-1` | exit 0 | — |
| POST /user | `jira user create --email x@x.com` | exit 0 | — |
| DELETE /user | `jira user delete acc123` | exit 0 | — |

#### Groups (16 tests)

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| GET /groups/picker | `jira group list` | exit 0 | — |
| GET /groups/picker (search) | `jira group search "dev"` | exit 0 | — |
| GET /group | `jira group get "developers"` | exit 0 | 404 |
| POST /group | `jira group create "new-group"` | exit 0 | — |
| DELETE /group | `jira group delete "old-group"` | exit 0 | — |
| GET /group/member | `jira group members "developers"` | exit 0 | — |
| POST /group/user | `jira group add-user "grp" acc123` | exit 0 | — |
| DELETE /group/user | `jira group remove-user "grp" acc123` | exit 0 | — |

#### Filters (14 tests)

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| GET /filter/favourite | `jira filter list --favourites` | exit 0 | — |
| GET /filter/my | `jira filter list --mine` | exit 0 | — |
| GET /filter/search | `jira filter list --name "test"` | exit 0 | — |
| GET /filter/{id} | `jira filter get 10001` | exit 0 | 404 |
| POST /filter | `jira filter create --name "F" --jql "..."` | exit 0 | — |
| PUT /filter/{id} | `jira filter update 10001 --name "New"` | exit 0 | — |
| DELETE /filter/{id} | `jira filter delete 10001` | exit 0 | read-only |

#### Dashboards (20 tests)

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| GET /dashboard | `jira dashboard list` | exit 0 | — |
| GET /dashboard/{id} | `jira dashboard get 10001` | exit 0 | 404 |
| POST /dashboard | `jira dashboard create --name "D"` | exit 0 | — |
| PUT /dashboard/{id} | `jira dashboard update 10001 --name "X"` | exit 0 | — |
| DELETE /dashboard/{id} | `jira dashboard delete 10001` | exit 0 | — |
| POST /dashboard/{id}/copy | `jira dashboard copy 10001` | exit 0 | — |
| GET gadgets | `jira dashboard gadgets 10001` | exit 0 | — |
| POST gadget | `jira dashboard add-gadget 10001 --uri x` | exit 0 | — |
| PUT gadget | `jira dashboard update-gadget 10001 20001` | exit 0 | — |
| DELETE gadget | `jira dashboard remove-gadget 10001 20001` | exit 0 | — |

#### Versions & Components (22 tests)

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| GET /project/{k}/versions | `jira version list TEST` | exit 0 | — |
| GET /version/{id} | `jira version get 10001` | exit 0 | 404 |
| POST /version | `jira version create --project TEST --name "1.0"` | exit 0 | — |
| PUT /version/{id} | `jira version update 10001 --name "1.1"` | exit 0 | — |
| DELETE /version/{id} | `jira version delete 10001` | exit 0 | — |
| PUT /version/{id} (release) | `jira version release 10001` | exit 0 | — |
| GET /project/{k}/components | `jira component list TEST` | exit 0 | — |
| GET /component/{id} | `jira component get 10001` | exit 0 | 404 |
| POST /component | `jira component create --project TEST --name "BE"` | exit 0 | — |
| PUT /component/{id} | `jira component update 10001 --name "FE"` | exit 0 | — |
| DELETE /component/{id} | `jira component delete 10001` | exit 0 | — |

#### Fields (10 tests)

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| GET /field | `jira field list` | exit 0 | — |
| POST /field | `jira field create --name "F" --type "..."` | exit 0 | — |
| DELETE /field/{id} | `jira field delete cf_10001` | exit 0 | — |
| POST /field/{id}/trash | `jira field trash cf_10001` | exit 0 | — |
| POST /field/{id}/restore | `jira field restore cf_10001` | exit 0 | — |

#### Admin — Issue Types (10 tests)

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| GET /issuetype | `jira issue-type list` | exit 0 | — |
| GET /issuetype/{id} | `jira issue-type get 10001` | exit 0 | 404 |
| POST /issuetype | `jira issue-type create --name "Bug"` | exit 0 | — |
| PUT /issuetype/{id} | `jira issue-type update 10001 --name "X"` | exit 0 | — |
| DELETE /issuetype/{id} | `jira issue-type delete 10001` | exit 0 | — |

#### Admin — Priorities (10 tests)

Same CRUD pattern as Issue Types:
`jira priority list/get/create/update/delete`

#### Admin — Resolutions (10 tests)

Same CRUD pattern:
`jira resolution list/get/create/update/delete`

#### Admin — Statuses (6 tests)

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| GET /status | `jira status list` | exit 0 | — |
| GET /status/{id} | `jira status get 10001` | exit 0 | 404 |
| GET /statuscategory | `jira status categories` | exit 0 | — |

#### Admin — Screens (12 tests)

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| GET /screens | `jira screen list` | exit 0 | — |
| GET /screens/{id} | `jira screen get 10001` | exit 0 | — |
| POST /screens | `jira screen create --name "S"` | exit 0 | — |
| DELETE /screens/{id} | `jira screen delete 10001` | exit 0 | — |
| GET tabs | `jira screen tabs 10001` | exit 0 | — |
| GET tab fields | `jira screen fields 10001 20001` | exit 0 | — |

#### Admin — Schemes (CRUD x5 = 50 tests)

Each scheme type follows the same CRUD pattern (list/get/create/update/delete):
- `jira workflow-scheme list/get/create/update/delete`
- `jira permission-scheme list/get/create/update/delete`
- `jira notification-scheme list/get/create/update/delete`
- `jira issue-security-scheme list/get/create/update/delete`
- `jira issue-type-scheme list/get/create/update/delete`

Plus `jira workflow list/get` (read-only, 4 tests)

#### Admin — Field Configurations (8 tests)

`jira field-config list/get/create/delete`

#### Admin — Roles (8 tests)

`jira role list/get/create/delete`

#### Admin — Project Categories (10 tests)

`jira project-category list/get/create/update/delete`

#### Admin — Banner (4 tests)

`jira banner get` / `jira banner set --message "X"`

#### Admin — Tasks (4 tests)

`jira task get ID` / `jira task cancel ID`

#### Admin — Attachments (6 tests)

`jira attachment get/delete/meta`

#### Admin — Misc (14 tests)

`jira labels`, `jira server-info`, `jira configuration`, `jira permissions`, `jira my-permissions`, `jira audit-records`

#### Admin — Webhooks (8 tests)

`jira webhook list/get/create/delete`

---

### 4.2 Jira Agile (`contract_jira_agile.rs`)

**22 operations, ~44 test cases**

#### Boards (12 tests)

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| GET /board | `jira board list` | exit 0 | — |
| GET /board (filtered) | `jira board list --project TEST` | exit 0 | — |
| GET /board/{id} | `jira board get 1` | exit 0 | 404 |
| GET /board/{id}/configuration | `jira board config 1` | exit 0 | — |
| GET /board/{id}/issue | `jira board issues 1` | exit 0 | — |
| GET /board/{id}/backlog | `jira board backlog 1` | exit 0 | — |

#### Sprints (18 tests)

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| GET /board/{id}/sprint | `jira sprint list 1` | exit 0 | — |
| GET /board/{id}/sprint (state) | `jira sprint list 1 --state active` | exit 0 | — |
| GET /sprint/{id} | `jira sprint get 1` | exit 0 | 404 |
| GET /sprint/{id}/issue | `jira sprint issues 1` | exit 0 | — |
| POST /sprint | `jira sprint create --board-id 1 --name "S1"` | exit 0 | — |
| PUT /sprint/{id} | `jira sprint update 1 --name "S2"` | exit 0 | — |
| DELETE /sprint/{id} | `jira sprint delete 1` | exit 0 | — |
| POST /sprint/{id}/issue | `jira sprint move 1 TEST-1 TEST-2` | exit 0 | — |
| POST /backlog/issue | `jira backlog-move TEST-1 TEST-2` | exit 0 | — |

#### Epics (10 tests)

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| GET /board/{id}/epic | `jira epic list 1` | exit 0 | — |
| GET /epic/{key} | `jira epic get TEST-100` | exit 0 | 404 |
| GET /epic/{key}/issue | `jira epic issues TEST-100` | exit 0 | — |
| POST /epic/{key}/issue | `jira epic add TEST-100 TEST-1 TEST-2` | exit 0 | — |
| POST /epic/none/issue | `jira epic remove TEST-1` | exit 0 | — |

---

### 4.3 Confluence v1 (`contract_confluence_v1.rs`)

**Only the 6 methods still using v1 endpoints. ~16 test cases.**

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| GET /content/search | `conf search "type=page"` | exit 0 | invalid CQL |
| GET /content/search (all) | `conf search "type=page" --all` | exit 0 | — |
| GET /content/search (paginated) | `conf search "space=DEV" --limit 5` | exit 0 | — |
| GET /content/{id}/download | `conf attachment download 12345` | exit 0 | 404 |
| POST /content/{id}/child/attachment | `conf attachment upload 12345 -f /tmp/t.txt` | exit 0 | read-only |
| POST /content/{id}/label | `conf label add 12345 tag1 tag2` | exit 0 | read-only |
| DELETE /content/{id}/label/{l} | `conf label remove 12345 tag1` | exit 0 | read-only |

> **Note:** All other Confluence methods (pages CRUD, spaces, children, comments, attachments list/delete, properties, blog posts) have been migrated to v2 and are tested in `contract_confluence_v2.rs`.

---

### 4.4 Confluence v2 (`contract_confluence_v2.rs`)

**~200 operations, ~400 test cases.** Organized by resource group.

> **Important:** This section includes methods migrated from v1 (marked with ⇐v1). These are the primary tests for page CRUD, children, comments, attachments, properties, and blog posts — they now hit the v2 Prism instance.

#### Spaces (26 tests)

| Operation | CLI Command | Positive | Negative |
|-----------|-------------|----------|----------|
| GET /spaces | `conf space list` | exit 0 | — |
| GET /spaces/{id} | `conf space get 12345` | exit 0 | 404 |
| POST /spaces | `conf space create -k TEST -n "Test"` | exit 0 | missing key |
| DELETE /spaces/{id} | `conf space delete 12345` | exit 0 | read-only |
| GET /spaces/{id}/pages | `conf space pages 12345` | exit 0 | — |
| GET /spaces/{id}/blogposts | `conf space blogposts 12345` | exit 0 | — |
| GET /spaces/{id}/labels | `conf space labels 12345` | exit 0 | — |
| GET /spaces/{id}/permissions | `conf space permissions 12345` | exit 0 | — |
| GET /space-permissions | `conf space permissions-available` | exit 0 | — |
| GET /spaces/{id}/content/labels | `conf space content-labels 12345` | exit 0 | — |
| GET /spaces/{id}/custom-content | `conf space custom-content 12345 -t type` | exit 0 | — |
| GET /spaces/{id}/operations | `conf space operations 12345` | exit 0 | — |
| GET/POST /spaces/{id}/role-assignments | role assignment CRUD | exit 0 | — |

#### Pages (32 tests) — includes ⇐v1 migrated methods

| Operation | CLI Command | Positive | Negative | Note |
|-----------|-------------|----------|----------|------|
| GET /pages/{id} | `conf read 12345` | exit 0 | 404 | ⇐v1 `get_page` |
| GET /pages/{id} (view) | `conf read 12345 --body-format view` | exit 0 | — | ⇐v1 body-format param |
| GET /pages/{id} (info) | `conf info 12345` | exit 0 | 404 | ⇐v1 `get_page_info` |
| POST /pages | `conf create -s TEST -t "Title" -b "body"` | exit 0 | missing space | ⇐v1 `create_page` |
| POST /pages (markdown) | `conf create -s TEST -t "T" --input-format markdown -b "# H"` | exit 0 | — | ⇐v1 |
| POST /pages (parent) | `conf create -s TEST -t "T" -b "b" --parent 111` | exit 0 | — | ⇐v1 |
| PUT /pages/{id} | `conf update 12345 -t "T" -b "B" --version 2` | exit 0 | read-only | ⇐v1 `update_page` |
| DELETE /pages/{id} | `conf delete 12345` | exit 0 | read-only | ⇐v1 `delete_page` |
| DELETE /pages/{id} (purge) | `conf delete 12345 --purge` | exit 0 | — | ⇐v1 |
| GET /pages/{id}/children | `conf children 12345` | exit 0 | — | ⇐v1 `get_children` |
| GET /pages/{id}/children (tree) | `conf tree 12345 --depth 2` | exit 0 | — | ⇐v1 `get_children_recursive` |
| GET /pages | `conf page-list` | exit 0 | — | |
| GET /pages/{id}/ancestors | `conf ancestors 12345` | exit 0 | — | |
| GET /pages/{id}/descendants | `conf descendants 12345` | exit 0 | — | |
| GET /pages/{id}/footer-comments | `conf footer-comment list 12345` | exit 0 | — | |
| GET /pages/{id}/inline-comments | `conf inline-comment list 12345` | exit 0 | — | |
| GET /pages/{id}/versions | `conf versions 12345` | exit 0 | — | |
| GET /pages/{id}/likes/count | `conf likes-count 12345` | exit 0 | — | |
| GET /pages/{id}/likes/users | `conf likes-users 12345` | exit 0 | — | |
| GET /pages/{id}/operations | `conf operations 12345` | exit 0 | — | |
| PUT /pages/{id}/title | `conf update-title 12345 -t "New" --version 2` | exit 0 | — | |
| POST /pages/{id}/redact | `conf redact 12345` | exit 0 | — | |
| GET /pages/{id}/custom-content | `conf page-custom-content 12345 -t type` | exit 0 | — | |
| GET/POST /pages/{page-id}/properties | property CRUD | exit 0 | — | |

#### Page Properties (12 tests) — ⇐v1 migrated

| Operation | CLI Command | Positive | Negative | Note |
|-----------|-------------|----------|----------|------|
| GET /pages/{id}/properties | `conf property list 12345` | exit 0 | — | ⇐v1 `get_properties` |
| GET /pages/{id}/properties (by key) | `conf property get 12345 mykey` | exit 0 | not found | ⇐v1 `get_property` |
| POST /pages/{id}/properties | `conf property set 12345 mykey '{"v":1}'` | exit 0 | read-only | ⇐v1 `set_property` (create) |
| PUT /pages/{id}/properties/{pid} | `conf property set 12345 mykey '{"v":2}'` | exit 0 | — | ⇐v1 `set_property` (update) |
| DELETE /pages/{id}/properties/{pid} | `conf property delete 12345 mykey` | exit 0 | not found | ⇐v1 `delete_property` |

#### Attachments (20 tests) — partially ⇐v1 migrated

| Operation | CLI Command | Positive | Negative | Note |
|-----------|-------------|----------|----------|------|
| GET /pages/{id}/attachments | `conf attachment list 12345` | exit 0 | — | ⇐v1 `get_attachments` |
| GET /pages/{id}/attachments (filter) | `conf attachment list 12345 --media-type image/png` | exit 0 | — | ⇐v1 |
| DELETE /attachments/{id} | `conf attachment delete 67890` | exit 0 | read-only | ⇐v1 `delete_attachment` |
| GET /attachments/{id}/labels | `conf attachment labels 12345` | exit 0 | — | |
| GET /attachments/{id}/comments | `conf attachment comments 12345` | exit 0 | — | |
| GET /attachments/{id}/operations | `conf attachment operations 12345` | exit 0 | — | |
| GET /attachments/{id}/versions | `conf attachment versions 12345` | exit 0 | — | |
| GET/POST /attachments/{id}/properties | attachment property CRUD | exit 0 | — | |

> Note: `upload_attachment` and `download_attachment` still use v1 — tested in `contract_confluence_v1.rs`.

#### Footer Comments (22 tests) — partially ⇐v1 migrated

| Operation | CLI Command | Positive | Negative | Note |
|-----------|-------------|----------|----------|------|
| GET /pages/{id}/footer-comments | `conf comment list 12345` | exit 0 | — | ⇐v1 `get_comments` |
| POST /footer-comments | `conf comment add 12345 -b "text"` | exit 0 | read-only | ⇐v1 `create_comment` |
| POST /footer-comments (reply) | `conf comment add 12345 -b "re" --parent 999` | exit 0 | — | ⇐v1 |
| DELETE /footer-comments/{id} | `conf comment delete 999` | exit 0 | read-only | ⇐v1 `delete_comment` |
| GET /footer-comments/{id}/children | comment children | exit 0 | — | |
| GET /footer-comments/{id}/versions | comment versions | exit 0 | — | |
| GET /footer-comments/{id}/likes | comment likes | exit 0 | — | |
| GET /footer-comments/{id}/operations | comment operations | exit 0 | — | |

#### Blog Posts (~34 tests) — ⇐v1 migrated

| Operation | CLI Command | Positive | Negative | Note |
|-----------|-------------|----------|----------|------|
| GET /blogposts | `conf blog list` | exit 0 | — | ⇐v1 `list_blog_posts` |
| GET /blogposts (space) | `conf blog list --space TEST` | exit 0 | — | ⇐v1 |
| GET /blogposts/{id} | `conf blog read 12345` | exit 0 | 404 | ⇐v1 `get_blog_post` |
| POST /blogposts | `conf blog create -s TEST -t "T" -b "B"` | exit 0 | read-only | ⇐v1 `create_blog_post` |
| PUT /blogposts/{id} | `conf blog update 12345 -t "T" -b "B" --version 2` | exit 0 | — | ⇐v1 `update_blog_post` |
| DELETE /blogposts/{id} | `conf blog delete 12345` | exit 0 | read-only | ⇐v1 `delete_blog_post` |

Plus sub-resources (attachments, labels, footer-comments, inline-comments, versions, likes, operations, custom-content, redact) + property CRUD

#### Footer Comments (22 tests)

`conf footer-comment list/get/create/update/delete/children/versions/likes/likes-count/likes-users/operations/version-details` + property CRUD

#### Inline Comments (22 tests)

`conf inline-comment list/get/create/update/delete/children/versions/likes/likes-count/likes-users/operations/version-details` + property CRUD

#### Attachments (18 tests)

`conf attachment list/get/upload/delete/download/labels/comments/operations/versions/version-details` + property CRUD

#### Content Types — Whiteboards, Databases, Folders, Smart Links (4 x 16 = 64 tests)

Each type: `create/get/delete/ancestors/descendants/children/operations` + property CRUD (list/get/set/delete)

#### Custom Content (22 tests)

`conf custom-content list/get/create/update/delete` + sub-resources (attachments, children, labels, comments, operations, versions, version-details) + property CRUD

#### Tasks (6 tests)

`conf task list/get/update`

#### Labels (8 tests)

`conf label pages/blogposts/attachments` by label ID

#### Admin — Admin Key, Data Policy, Classification (~16 tests)

- `conf admin-key get/enable/disable`
- `conf data-policy metadata/spaces`
- `conf classification list/get-page/set-page/reset-page/get-blogpost/set-blogpost/reset-blogpost/...`

#### Users (6 tests)

`conf user bulk/check-access/invite`

#### Misc (8 tests)

`conf convert-ids`, `conf app-property list/get/set/delete`

---

## 5. Cross-Cutting Tests (`contract_cross_cutting.rs`)

```rust
#[test] fn auth_error_jira()        // bad token → exit 4
#[test] fn auth_error_confluence()   // bad token → exit 4
#[test] fn no_config()               // missing config → exit 3
#[test] fn bad_profile()             // nonexistent profile → exit 3
#[test] fn output_format_json()      // --format json → valid JSON
#[test] fn output_format_csv()       // --format csv → valid CSV
#[test] fn output_format_toon()      // --format toon → valid TOON
#[test] fn quiet_mode()              // -q suppresses output
#[test] fn verbose_mode()            // -vvv enables tracing
#[test] fn read_only_blocks_writes() // all write ops fail with exit 3
```

---

## 6. Key Design Decisions

1. **`#[ignore]` over `cfg(feature)`** — tests always compile (catching type drift), only run with `cargo test -- --ignored`
2. **Dual Prism for Confluence** — separate v1 and v2 Prism instances in separate test files; no reverse proxy
3. **`LazyLock` statics** — one Prism + AtlRunner per test file, shared across all tests
4. **`assert_cmd` + `tempfile`** — ergonomic dev-dependencies for test helpers
5. **Delegation** — `/rust:cli` skill → `rust-cli-writer` subagent for each Rust implementation task

---

## 7. Implementation Order

### Phase 1: Foundation (Tasks 1-6)

| # | Task | Files | Delegate | Depends |
|---|------|-------|----------|---------|
| 1 | Cargo.toml + .gitignore | `Cargo.toml`, `.gitignore` | Direct | — |
| 2 | Spec download script | `tests/contract/download_specs.sh` | Direct | — |
| 3 | Spec patching script | `tests/contract/patch_specs.py` | Direct | 2 |
| 4 | PrismServer module | `tests/common/prism.rs` | `/rust:cli` → `rust-cli-writer` | 1 |
| 5 | TestConfig module | `tests/common/config.rs` | `/rust:cli` → `rust-cli-writer` | 4 |
| 6 | AtlRunner + mod.rs | `tests/common/atl.rs`, `tests/common/mod.rs` | `/rust:cli` → `rust-cli-writer` | 5 |

### Phase 2: Jira Tests (Tasks 7-14, independent after Task 6)

| # | Task | Tests | File |
|---|------|-------|------|
| 7 | Issues Core CRUD | 32 | `contract_jira_platform.rs` (create) |
| 8 | Issues Extras (worklog, watchers, vote, links, clone) | 44 | `contract_jira_platform.rs` (append) |
| 9 | Projects + Users | 34 | `contract_jira_platform.rs` (append) |
| 10 | Groups + Filters + Dashboards | 50 | `contract_jira_platform.rs` (append) |
| 11 | Versions + Components + Fields | 42 | `contract_jira_platform.rs` (append) |
| 12 | Admin Types/Priorities/Statuses/Screens | 48 | `contract_jira_platform.rs` (append) |
| 13 | Admin Schemes + Misc | 74 | `contract_jira_platform.rs` (append) |
| 14 | Jira Agile (boards, sprints, epics) | 44 | `contract_jira_agile.rs` (create) |

### Phase 3: Confluence Tests (Tasks 15-17, independent after Task 6)

| # | Task | Tests | File |
|---|------|-------|------|
| 15 | Confluence v1 (search, upload/download, labels) | 16 | `contract_confluence_v1.rs` (create) |
| 16 | v2 Part A: Pages + Properties + Attachments + Comments | 86 | `contract_confluence_v2.rs` (create) |
| 17a | v2 Part B: Inline Comments + Blog Posts | 56 | `contract_confluence_v2.rs` (append) |
| 17b | v2 Part C: Spaces + Content Types | 90 | `contract_confluence_v2.rs` (append) |
| 17c | v2 Part D: Custom Content + Tasks + Labels + Admin + Misc | 66 | `contract_confluence_v2.rs` (append) |

### Phase 4: Cross-Cutting (Task 18, independent after Task 6)

| # | Task | Tests | File |
|---|------|-------|------|
| 18 | Auth, config, output formats, read-only, verbosity | 10 | `contract_cross_cutting.rs` (create) |

**Total: ~830 test cases across 5 test files**

---

## 8. Verification

```bash
# Download and patch specs
bash tests/contract/download_specs.sh
python3 tests/contract/patch_specs.py

# Verify all tests compile
cargo test --no-run 2>&1 | tail -5

# Run all contract tests (ignored tests only)
cargo test -- --ignored --test-threads=4

# Run specific test file
cargo test --test contract_jira_platform -- --ignored

# Run single test
cargo test --test contract_jira_platform -- --ignored jira_search_issues_positive
```

### CI integration
- Install: `npm install -g @stoplight/prism-cli`
- Gate: `#[ignore]` — run with `cargo test -- --ignored`
- Tests always compile during normal `cargo test` (catches type drift)

---

## 9. Files to Create/Modify

| File | Action |
|------|--------|
| `Cargo.toml` | Add `[dev-dependencies]`: assert_cmd, predicates, tempfile |
| `.gitignore` | Add `tests/contract/specs/` and `*.patched.json` |
| `tests/contract/download_specs.sh` | Create — idempotent spec download |
| `tests/contract/patch_specs.py` | Create — spec /api/3/ → /api/2/ rewrite |
| `tests/common/mod.rs` | Create — re-exports |
| `tests/common/prism.rs` | Create — PrismServer struct |
| `tests/common/atl.rs` | Create — AtlRunner struct |
| `tests/common/config.rs` | Create — TestConfig builder |
| `tests/contract_jira_platform.rs` | Create — ~324 test cases |
| `tests/contract_jira_agile.rs` | Create — ~44 test cases |
| `tests/contract_confluence_v1.rs` | Create — ~16 test cases (v1 methods only) |
| `tests/contract_confluence_v2.rs` | Create — ~430 test cases (includes migrated v1 methods) |
| `tests/contract_cross_cutting.rs` | Create — ~10 test cases |

---

## 10. Coding Plan — Delegation Workflow

### Prerequisites

Before each Rust coding task:
1. Invoke skill `/rust:cli` to load Rust CLI conventions
2. Delegate to `Agent(subagent_type: "rust-cli-writer")` with conventions + task context
3. Include file paths, test matrices, and infrastructure API in the prompt

### Infrastructure Details

**PrismServer** (`tests/common/prism.rs`):
```rust
pub struct PrismServer { process: Child, port: u16, base_url: String }

impl PrismServer {
    pub fn start(spec_path: &str) -> Self;   // find free port, spawn npx prism, wait ready
    pub fn base_url(&self) -> &str;
    pub fn port(&self) -> u16;
}
impl Drop for PrismServer { fn drop(&mut self) { kill + wait } }
```

**TestConfig** (`tests/common/config.rs`):
```rust
TestConfigBuilder::new()
    .jira("http://127.0.0.1:PORT")
    .confluence("http://127.0.0.1:PORT")
    .confluence_api_path("/wiki/rest/api")  // client derives v2 via .replace("/rest/api", "/api/v2")
    .read_only(false)
    .build() -> TestConfig { _dir: TempDir, config_path: PathBuf }
```

**AtlRunner** (`tests/common/atl.rs`):
```rust
pub struct AtlRunner { config_path: PathBuf }
pub struct AtlResult { pub exit_code: i32, pub stdout: String, pub stderr: String }

impl AtlRunner {
    pub fn new(config_path: &Path) -> Self;
    pub fn run(&self, args: &[&str]) -> AtlResult;     // via assert_cmd::Command::cargo_bin("atl")
    pub fn run_ok(&self, args: &[&str]) -> String;      // assert exit 0, return stdout
    pub fn run_err(&self, args: &[&str], code: i32) -> String;  // assert exit == code
}
```

**Static pattern** (each test file):
```rust
mod common;
use std::sync::LazyLock;

static PRISM: LazyLock<PrismServer> = LazyLock::new(|| {
    PrismServer::start("tests/contract/specs/jira-platform.patched.json")
});
static SETUP: LazyLock<(TestConfig, AtlRunner)> = LazyLock::new(|| {
    let config = TestConfigBuilder::new().jira(PRISM.base_url()).build();
    let runner = AtlRunner::new(&config.config_path);
    (config, runner)
});
fn runner() -> &'static AtlRunner { &SETUP.1 }

// Read-only config for negative tests:
static SETUP_RO: LazyLock<(TestConfig, AtlRunner)> = LazyLock::new(|| {
    let config = TestConfigBuilder::new().jira(PRISM.base_url()).read_only(true).build();
    let runner = AtlRunner::new(&config.config_path);
    (config, runner)
});
fn runner_ro() -> &'static AtlRunner { &SETUP_RO.1 }
```

**Test function pattern**:
```rust
#[test]
#[ignore]
fn jira_search_issues_positive() {
    let out = runner().run_ok(&["jira", "search", "project=TEST"]);
    assert!(!out.is_empty());
}

#[test]
#[ignore]
fn jira_update_issue_read_only() {
    runner_ro().run_err(&["jira", "update", "TEST-1", "--summary", "X"], 3); // CONFIG_ERROR
}
```

### Delegation Template

```text
Skill loaded: /rust:cli (conventions follow)
{skill output}

Task: Implement contract tests for {scope}.

Context:
- Project: atl — Rust CLI for Atlassian (edition 2024, MSRV 1.94)
- Test approach: Prism mock server validates HTTP requests against OpenAPI specs
- Infrastructure: tests/common/{prism.rs, atl.rs, config.rs}
- Exit codes: SUCCESS=0, RUNTIME=1, NOT_FOUND=2, CONFIG=3, AUTH=4
- File to create/modify: {file_path}

Requirements:
- {test matrix from section 4.N}
- All test functions: #[test] #[ignore]
- LazyLock for shared PrismServer + AtlRunner
- Positive: run_ok(), assert non-empty output
- Negative: run_err(code), read-only via SETUP_RO
```
