# Contributing to atl

Thanks for your interest in improving `atl`. This document covers everything you need to build, test, and submit changes.

## Ground rules

Before you open a pull request:

- **Open an issue first for non-trivial changes.** Bug fixes and small improvements can go straight to a PR, but for new subcommands, new output formats, or changes to the public CLI surface, please file an issue so we can agree on scope and UX before you invest time.
- **Keep changes focused.** One logical change per PR. A bug fix should not also refactor unrelated code; a new command should not also bump unrelated dependencies.
- **Do not expand scope unprompted.** Don't add features that were not requested. Don't rewrite code you didn't need to touch. Don't add backwards-compat shims for code paths that don't exist yet.
- **No breaking CLI changes without discussion.** `atl` is a tool people script against. Flag renames, removed commands, and changes to default output shape need an issue first.

## Prerequisites

- **Rust stable, MSRV 1.94** (see `rust-version` in `Cargo.toml`). Install via [rustup](https://rustup.rs/).
- A POSIX-like shell for the release helper scripts (`scripts/bump-version.sh`). On Windows, WSL or Git Bash is fine; the CLI itself builds and runs on native Windows.
- Optional: [lefthook](https://github.com/evilmartians/lefthook) for pre-commit hooks. Install with `brew install lefthook` (or equivalent), then run `lefthook install` once inside the repo.

## Build and test

```sh
cargo check                         # fast type-check
cargo build                         # debug build at target/debug/atl
cargo build --release               # optimised binary at target/release/atl
cargo test                          # unit + integration tests
cargo test <name>                   # run a single test by substring match
cargo test -- --nocapture           # see stdout from tests
cargo fmt --all                     # format
cargo fmt --all -- --check          # check formatting without modifying
cargo clippy --all-targets -- -D warnings   # same lint the CI enforces
```

CI runs the exact commands above on Linux, macOS, and Windows plus an MSRV check — see [`.github/workflows/ci.yml`](workflows/ci.yml). Anything green locally should be green in CI.

### Pre-commit hooks

If you installed lefthook, every commit automatically runs `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo test` in parallel. Configuration lives in [`lefthook.yml`](../lefthook.yml).

### Integration and contract tests

End-to-end contract tests live under `tests/` and use [Prism](https://github.com/stoplightio/prism) to mock Atlassian APIs against the official OpenAPI specs. See [`docs/CONTRACT_TESTS_PLAN.md`](../docs/CONTRACT_TESTS_PLAN.md) for the approach. If you change a command that touches the wire protocol, add or update a contract test for it.

## Project layout

See [`docs/project-layout.md`](../docs/project-layout.md) for a directory tour and a step-by-step walkthrough of adding a new Jira subcommand. The short version:

```text
src/
├── main.rs               # parse args, init logging, dispatch
├── cli/args/             # clap derive structs (one file per service)
├── cli/commands/         # command handlers (one file per service)
├── client/               # HTTP clients for Confluence and Jira
├── config/               # TOML profile loading
├── output/               # Reporter trait + format implementations
└── error.rs              # domain Error enum + exit codes
```

## Pull request process

1. **Fork and branch.** Branch from `master`. Name branches descriptively: `fix/jira-search-pagination`, `feat/confluence-labels`.
2. **Write tests.** Add unit tests next to the code you change, and a contract/integration test if you touched the wire protocol. A PR that changes behaviour without tests will be asked to add them.
3. **Run the full local check** before pushing: `cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test`.
4. **Write a Conventional Commit message.** The project follows [Conventional Commits](https://www.conventionalcommits.org/) — recent history shows the allowed types:
   - `feat:` – user-visible new capability
   - `fix:` – bug fix
   - `refactor:` – code change that does not alter behaviour
   - `test:` – adding or restructuring tests
   - `docs:` – documentation only
   - `ci:` – CI configuration
   - `release:` – version bumps (produced by `scripts/bump-version.sh`)
5. **Open the PR.** Fill out the template: summary, linked issue (`Fixes #NN` if applicable), and how you tested. Keep the title short — use the description for detail.
6. **Respond to review.** Push follow-up commits rather than force-pushing, unless a reviewer explicitly asks for a rebase. Mark conversations resolved as you address them.
7. **CI must be green.** The `ci-success` job gates the PR; if any matrix job fails, fix the underlying issue rather than retrying.

## Commit messages

Recent commits to follow as examples:

```text
feat: `atl self check` and `atl self update` from GitHub Releases (#6)
ci: add GitHub Actions workflows and YYYY.WW.BUILD release tooling (#5)
test: add E2E contract tests with Prism mock server (#3)
feat: unified Atlassian CLI for Confluence and Jira (#1)
```

Keep the subject line under ~72 characters. Explain *why* in the body if the change is not obvious from the diff.

## Reporting bugs and requesting features

- **Bug?** Use the [bug report template](ISSUE_TEMPLATE/bug_report.md) and include `atl --version`, steps to reproduce, expected vs actual, and logs (set `RUST_LOG=atl=debug` for verbose tracing).
- **Feature?** Use the [feature request template](ISSUE_TEMPLATE/feature_request.md). Describe the use case before the implementation — "I want to list all Jira filters I own" is more useful than "add `atl jira filter list`".
- **Security?** Do **not** open a public issue. See [`SECURITY.md`](SECURITY.md) for the private disclosure process.
- **Question?** Prefer GitHub Discussions over Issues for usage questions.

## Code of Conduct

Participation in this project is governed by the [Code of Conduct](CODE_OF_CONDUCT.md). By contributing you agree to abide by its terms.
