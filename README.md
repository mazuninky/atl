# atl

[![CI](https://github.com/mazuninky/atl/actions/workflows/ci.yml/badge.svg)](https://github.com/mazuninky/atl/actions/workflows/ci.yml)
[![Release](https://github.com/mazuninky/atl/actions/workflows/release.yml/badge.svg)](https://github.com/mazuninky/atl/actions/workflows/release.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![MSRV](https://img.shields.io/badge/MSRV-1.94-blue)](Cargo.toml)
![CodeRabbit Pull Request Reviews](https://img.shields.io/coderabbit/prs/github/mazuninky/atl?utm_source=oss&utm_medium=github&utm_campaign=mazuninky%2Fatl&labelColor=171717&color=FF570A&link=https%3A%2F%2Fcoderabbit.ai&label=CodeRabbit+Reviews)

Unified command-line interface for Atlassian **Confluence** and **Jira**. Written in Rust, non-interactive by design, with structured output and multi-profile config.

- Works with both **Cloud** and **Data Center / Server** instances.
- Speaks Confluence REST API v1 and v2 (auto-probes the right path).
- Speaks Jira REST API v2 plus the Jira Agile API (boards, sprints, epics).
- Structured output: `console`, `json`, `toon`, `toml`, `csv`.
- Multiple named profiles, env-var overrides, no secrets in flags.
- A generic `atl api` passthrough for any endpoint the dedicated commands do not cover.

## Installation

### From GitHub Releases (recommended)

```sh
curl -sSfL https://raw.githubusercontent.com/mazuninky/atl/master/scripts/install.sh | sh
```

To install a specific version:

```sh
curl -sSfL https://raw.githubusercontent.com/mazuninky/atl/master/scripts/install.sh | sh -s -- --version 2026.15.1
```

Prebuilt binaries are available for Linux (x86_64), macOS (arm64), and Windows (x86_64). The script installs to `/usr/local/bin` by default; use `--install-dir DIR` to override. On Windows, download from the [releases page](https://github.com/mazuninky/atl/releases/latest) manually.

### From source

Requires Rust stable **1.94 or newer**.

```sh
git clone https://github.com/mazuninky/atl.git
cd atl
cargo install --path .
```

On macOS, ad-hoc sign the binary so it can access the login keychain without password prompts:

```sh
codesign -s - -f ~/.cargo/bin/atl
```

### Verifying release artifacts

Starting with the next release, every `atl-*.tar.gz` / `atl-*.zip` published to GitHub Releases is signed via [SLSA build provenance](https://slsa.dev/). Verify with the `gh` CLI:

```sh
gh attestation verify atl-2026.16.2-x86_64-unknown-linux-gnu.tar.gz --repo mazuninky/atl
```

The attestation proves the archive was built by this repo's `release.yml` workflow at a specific tag, signed by a Sigstore short-lived certificate tied to GitHub's OIDC identity.

### Self-update

Once installed, `atl` can update itself from GitHub Releases:

```sh
atl self check            # report whether a newer release exists
atl self update            # download and replace the current binary
atl self update --to 2026.16.2   # pin to a specific version
```

## Quick start

```sh
# First-time setup: interactive profile wizard
atl init

# Confluence
atl confluence read 123456
atl confluence search "space = DEV AND type = page" --limit 10
atl confluence create --space DEV --title "Design notes" --body @notes.md --body-format markdown

# Jira
atl jira me
atl jira search "project = PROJ AND status = Open" --limit 20
atl jira view PROJ-123
atl jira create --project PROJ --issue-type Task --summary "Fix bug"
atl jira move PROJ-123 --transition 31
atl jira comment PROJ-123 --body "Done"

# Aliases: confluence/conf/c, jira/j
atl c read 123456
atl j view PROJ-123
```

Command-line syntax notation used throughout the help text is documented in [`docs/command-line-syntax.md`](docs/command-line-syntax.md).

## Configuration

Configuration lives in TOML at the platform-default location (on Linux/macOS: `~/.config/atl/atl.toml`). A profile groups credentials for one Confluence instance and/or one Jira instance:

```toml
default_profile = "work"

[profiles.work.confluence]
domain = "https://example.atlassian.net"
email = "me@example.com"
api_token = "…"            # Basic auth (Cloud)
# token  = "…"              # Bearer PAT (Data Center) — alternative to email+api_token
# read_only = true          # refuse any write operation

[profiles.work.jira]
domain = "https://example.atlassian.net"
email = "me@example.com"
api_token = "…"
```

Manage profiles without editing the file directly:

```sh
atl config list
atl config show work
atl config set-default work
atl config set-defaults --project PROJ --space DEV
```

Environment overrides:

| Variable | Purpose |
|---|---|
| `ATL_CONFIG` | Path to an alternate config file |
| `ATL_PROFILE` | Profile name to use (equivalent to `-p`) |
| `ATL_API_TOKEN` | Overrides the token stored in the profile |

## Output formats

All structured commands support the same global `-F` / `--format` flag:

```sh
atl -F json jira search "assignee = currentUser()"
atl -F toon confluence read 123456
atl -F csv jira search --status Open
atl -F toml config show work
```

`console` is the default and renders human-readable tables via `comfy-table`. Long output is piped through `$PAGER` by default; add `--no-pager` to disable.

## Generic REST passthrough

When a dedicated command does not exist, `atl api` gives you an authenticated `curl` over your profile:

```sh
atl api --service jira rest/api/2/myself
atl api --service jira rest/api/2/search \
    --query jql='project = TEST' --paginate
atl api --service confluence /wiki/api/v2/pages --query space-id=123 --paginate
atl api --service jira -X POST rest/api/2/issue \
    --raw-field 'fields={"project":{"key":"TEST"}}'
```

## Claude Code skill

An `atl` skill for [Claude Code](https://claude.ai/code) is available, giving Claude deep knowledge of all `atl` commands, flags, output formats, and common workflows.

Install from [skills.sh](https://skills.sh):

```sh
npx skills add mazuninky/atl
```

The skill covers Confluence and Jira commands, `atl api` passthrough, output pipelines (`-F json`, `--jq`, `--template`), CI/scripting patterns, aliases, authentication, and common pitfalls.

## Shell completions

```sh
atl completions bash > ~/.local/share/bash-completion/completions/atl
atl completions zsh  > "${fpath[1]}/_atl"
atl completions fish > ~/.config/fish/completions/atl.fish
```

## Contributing

Contributions are welcome. Start with [`.github/CONTRIBUTING.md`](.github/CONTRIBUTING.md) for build instructions, testing, and the pull-request workflow. Please report security issues privately — see [`.github/SECURITY.md`](.github/SECURITY.md).

For an overview of the source tree and how to add a new subcommand, see [`docs/project-layout.md`](docs/project-layout.md).

## License

Released under the [MIT License](LICENSE).
