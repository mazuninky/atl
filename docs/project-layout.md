# Project layout

This page is a map of the `atl` source tree for new contributors. It is meant to answer two questions quickly:

1. *Where does the code for X live?*
2. *How do I add a new subcommand without missing anything?*

If you just want to build and test, see [`.github/CONTRIBUTING.md`](../.github/CONTRIBUTING.md). For the notation used in command shapes, see [`command-line-syntax.md`](command-line-syntax.md).

## Directory tour

```
.
├── Cargo.toml                  # package metadata, deps, MSRV 1.94, edition 2024
├── CLAUDE.md                   # instructions for AI coding assistants
├── README.md                   # user-facing entry point
├── LICENSE                     # MIT
├── lefthook.yml                # optional pre-commit hooks (fmt/clippy/test)
├── clippy.toml                 # project clippy config
├── scripts/
│   └── bump-version.sh         # calendar version bump + tag (YYYY.WW.BUILD)
├── .github/
│   ├── workflows/ci.yml        # lint, test matrix, MSRV, release-smoke
│   ├── workflows/release.yml   # tag-triggered cross-platform build + GH Release
│   ├── CONTRIBUTING.md         # this project's contribution guide
│   ├── SECURITY.md             # private disclosure policy
│   ├── CODE_OF_CONDUCT.md      # Contributor Covenant 2.1 pointer
│   └── ISSUE_TEMPLATE/         # bug / feature templates + Discussions routing
├── docs/
│   ├── releasing.md            # operator checklist for cutting a release
│   ├── command-line-syntax.md  # CLI notation conventions
│   ├── project-layout.md       # this file
│   └── CONTRACT_TESTS_PLAN.md  # plan for Prism-backed E2E contract tests
├── src/
│   ├── main.rs                 # 3 jobs: parse args, init logging, dispatch
│   ├── lib.rs                  # library root, module re-exports
│   ├── error.rs                # domain Error enum, exit codes, Result alias
│   ├── io/                     # stdout/stderr streams + optional pager
│   ├── cli/
│   │   ├── args/               # clap derive structs (one file per service)
│   │   │   ├── mod.rs          # top-level Cli, Command enum, global flags
│   │   │   ├── api.rs          # `atl api` passthrough args
│   │   │   ├── confluence.rs   # ConfluenceSubcommand + arg structs
│   │   │   ├── jira.rs         # JiraSubcommand + arg structs
│   │   │   └── updater.rs      # `atl self check/update` args
│   │   └── commands/           # command handlers (one file per service)
│   │       ├── mod.rs          # read_body_arg helper (@file / - / literal)
│   │       ├── api.rs          # generic REST passthrough
│   │       ├── confluence.rs   # Confluence command dispatcher
│   │       ├── jira.rs         # Jira command dispatcher
│   │       ├── config.rs       # profile management (list/show/delete/...)
│   │       ├── init.rs         # interactive `atl init` wizard
│   │       ├── markdown.rs     # Markdown → Confluence storage format (comrak)
│   │       └── updater.rs      # self-update via GitHub Releases
│   ├── client/
│   │   ├── mod.rs              # shared HTTP client builder, auth, response handling
│   │   ├── confluence.rs       # ConfluenceClient (REST API v1 + v2, auto-probes path)
│   │   └── jira.rs             # JiraClient (REST API v2 + Agile API)
│   ├── config/
│   │   ├── mod.rs              # Config / Profile / AtlassianInstance (serde)
│   │   ├── loader.rs           # file discovery + env var overrides
│   │   └── default_config.toml # embedded default template
│   └── output/
│       ├── mod.rs              # Reporter trait + OutputFormat + factory
│       ├── console.rs          # human-readable tables (comfy-table)
│       ├── json.rs             # pretty JSON
│       ├── toon.rs             # TOON format
│       ├── toml_out.rs         # TOML
│       └── csv_out.rs          # CSV
└── tests/                      # E2E / contract tests (Prism mock server)
```

## Core patterns

A handful of conventions hold across the codebase. Learn these first and the rest of the tree will make sense.

### Thin `main.rs`, explicit runtime

`main.rs` does exactly three things: parse args with clap, initialise `tracing` based on `-v`/`-q`, and dispatch the `Command` enum. The Tokio runtime is built manually inside `run_async` rather than via `#[tokio::main]`, so sync commands (`init`, `config`, `completions`, `self check`) do not pay the cost of spawning a runtime. See `src/main.rs`.

### Errors: domain `Error` + `anyhow::Result`

The domain `Error` enum lives in `src/error.rs` and uses `thiserror`. Command handlers return `anyhow::Result<()>` so they can tack on context via `.context("doing X")`. On exit, `exit_code_for_error` downcasts the `anyhow::Error` back to the domain `Error` to pick the right process exit code (`CONFIG_ERROR=3`, `AUTH_ERROR=4`, `NOT_FOUND=2`, generic runtime error `1`).

Rule of thumb: raise a domain `Error::*` variant when you want a specific exit code; otherwise `anyhow::bail!` is fine.

### CLI args: derive, one file per service

Every subcommand is a `#[derive(Subcommand)]` variant with its own `Args` struct. Args live under `src/cli/args/`, split by service (`confluence.rs`, `jira.rs`, etc.). Global flags (`-v`, `-q`, `-F`, `-p`, `--config`, `--no-color`, `--no-pager`) are defined on the top-level `Cli` struct in `src/cli/args/mod.rs` with `global = true`.

### Command handlers: one file per service, dispatcher pattern

Each service has a `run(...)` entry point in `src/cli/commands/<service>.rs` that loads config, builds the client, and calls an internal `dispatch` that `match`es the enum. Adding a new subcommand means adding a variant to the enum, adding an arm to `dispatch`, and writing the actual logic as a private `async fn`.

### HTTP clients: construct from `AtlassianInstance`

`client/confluence.rs` and `client/jira.rs` each expose a `new(&AtlassianInstance, retries: u32) -> Result<Self>` that builds a middleware-wrapped `HttpClient` (`reqwest_middleware::ClientWithMiddleware`) with basic or bearer auth and an exponential-backoff `RetryTransientMiddleware` layer (attached when `retries > 0`). Callers must decide and pass the retry count — the shared `build_http_client` helper in `src/client/mod.rs` does the actual wiring. The `read_only` flag on the instance causes any non-GET request to be refused at the client layer — respect this in new code paths.

### Output: `Reporter` trait + `serde_json::Value`

Command handlers build a `serde_json::Value` and route rendering through the shared output pipeline by calling `write_output(value, format, io, transforms)`, which picks the current `Reporter` (chosen by `-F`) and delegates formatting to it. This means **you never print ad-hoc from a command handler**; the small set of documented exceptions (the `atl init` wizard and `atl auth login`/`atl auth status` console messages) write through `io.stdout()` directly because they are interactive or human-progress flows. The `Reporter` trait lives in `src/output/mod.rs`, and format implementations are sibling files.

### Body input: `@file`, `-`, or literal

Commands that take a body (Confluence `create`/`update`, Jira `comment`, `api --input`, …) accept three forms via the `read_body_arg` helper in `src/cli/commands/mod.rs`:

- A literal string — `--body "hello"`
- `@path` — read from a file
- `-` — read from stdin

Reuse `read_body_arg` rather than reimplementing.

### Non-interactive by design

No prompts, no spinners, no colour unless stdout is a TTY and `NO_COLOR` is unset. Commands must run identically under `| cat`, in CI, and from scripts. The only documented exceptions are the `atl init` wizard in `src/cli/commands/init.rs` and the `atl auth login` / `atl auth status` handlers in `src/cli/commands/auth.rs` — these intentionally prompt or print human-progress lines directly and sit outside the `write_output` pipeline.

## How to add a new Jira subcommand

Concrete walkthrough for adding, say, `atl jira filter list`:

1. **Define the args.** In `src/cli/args/jira.rs`, add a variant to `JiraSubcommand`:

   ```rust
   /// List saved filters
   Filter(JiraFilterCommand),
   ```

   Then declare the subcommand tree next to the other `*Command` structs in the same file:

   ```rust
   #[derive(Debug, Args)]
   pub struct JiraFilterCommand {
       #[command(subcommand)]
       pub command: JiraFilterSubcommand,
   }

   #[derive(Debug, Subcommand)]
   pub enum JiraFilterSubcommand {
       /// List filters owned by the current user
       List(JiraFilterListArgs),
   }

   #[derive(Debug, Args)]
   pub struct JiraFilterListArgs {
       /// Filter owner account ID (defaults to currentUser)
       #[arg(long, value_name = "ID")]
       pub owner: Option<String>,

       /// Maximum number of filters to return
       #[arg(long, default_value = "50")]
       pub limit: u32,
   }
   ```

2. **Add the client call.** In `src/client/jira.rs`, write an `async fn list_filters(...) -> Result<Value>` that builds the request URL, forwards to the shared request helper, and returns the parsed JSON.

3. **Wire up the dispatcher.** In `src/cli/commands/jira.rs`, extend the `match` in `dispatch` with a new arm:

   ```rust
   JiraSubcommand::Filter(cmd) => match &cmd.command {
       JiraFilterSubcommand::List(args) => {
           client.list_filters(args.owner.as_deref(), args.limit).await?
       }
   },
   ```

   The arm must produce a `serde_json::Value`; do not print directly.

4. **Document it.** Update `README.md` if the new command is user-visible, and extend `docs/command-line-syntax.md` only if you introduce a genuinely new notation shape.

5. **Test it.**
   - Unit-test any new pure helpers alongside the code.
   - If the command touches the wire protocol, add a contract test under `tests/` per [`docs/CONTRACT_TESTS_PLAN.md`](CONTRACT_TESTS_PLAN.md).
   - Smoke-test locally against a real instance or the Prism mock.

6. **Run the full local check** before opening a PR:

   ```sh
   cargo fmt --all -- --check
   cargo clippy --all-targets -- -D warnings
   cargo test
   ```

## Where not to put things

A few boundaries that are easy to get wrong:

- **Do not put HTTP calls in command handlers.** They belong in `src/client/`. The handler asks the client and renders the result.
- **Do not print from command handlers.** Build a `serde_json::Value` and hand it to `write_output` — the reporter handles formatting. `println!` and `eprintln!` belong only in `main.rs`, diagnostic logging paths (use `tracing::info!` etc.), and the documented interactive/progress exceptions: the `atl init` wizard and the `atl auth login` / `atl auth status` handlers.
- **Do not define global flags on subcommands.** Global flags live on the top-level `Cli` struct with `global = true`.
- **Do not widen `Error` casually.** Every new variant changes the exit-code mapping surface. Prefer adding context to `anyhow::Error` unless you need a distinct exit code.
