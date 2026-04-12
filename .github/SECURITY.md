# Security Policy

## Reporting a vulnerability

**Please do not open a public issue or pull request for security reports.**

If you believe you have found a security vulnerability in `atl`, report it privately through [GitHub Security Advisories](https://github.com/mazuninky/atl/security/advisories/new). This opens a confidential channel where we can discuss the details and coordinate a fix before any public disclosure.

If you cannot use GitHub Security Advisories, email **mazuninky@gmail.com** with:

- A description of the issue and its impact.
- Steps to reproduce, ideally with a minimal proof of concept.
- The affected `atl` version (`atl --version`) and the Atlassian product/version involved, if applicable.

You should receive an acknowledgement within a few days. We will work with you on a fix and a disclosure timeline, and credit you in the release notes unless you prefer to remain anonymous.

## Scope

`atl` is a thin REST client for Atlassian Confluence and Jira. Security-relevant surfaces are:

- **Credential handling.** API tokens, PATs, and email addresses stored in profile configuration files or environment variables.
- **HTTP client configuration.** TLS settings, request construction, response parsing.
- **File I/O used by commands.** Reading request bodies from `@file` or `-` (stdin), writing downloads, loading configuration.
- **Self-update mechanism.** Download and replacement of the running binary from GitHub Releases via `atl self update`.

Vulnerabilities in Confluence or Jira themselves are out of scope — report those directly to Atlassian.

## How `atl` handles secrets

A few properties worth knowing when evaluating impact:

- Credentials live in the profile config (default `~/.config/atl/atl.toml`) or are supplied via the `ATL_API_TOKEN` environment variable. They are **never** accepted via command-line flags, so they cannot be captured by shell history or process listings.
- Tokens are redacted from tracing output. Setting `RUST_LOG=atl=debug` or passing `-vv` will not print the token value, even on HTTP request traces.
- The `read_only` flag on a profile instance causes the client to refuse any non-GET operation regardless of which command is invoked, as a defence-in-depth measure for shared credentials.
- Self-update verifies it is replacing itself with an official GitHub Release asset for the `mazuninky/atl` repository.

If you find a case where a token leaks into logs, error messages, crash dumps, or stored state, please report it.

## Dependencies

We track dependency advisories via [`cargo audit`](https://github.com/rustsec/cargo-audit) and [Dependabot](./dependabot.yml). A CVE in one of our dependencies is **not** automatically a vulnerability in `atl`; whether it is exploitable depends on how we use the affected code path. When you report a dependency-based issue, please include a call chain or proof-of-concept showing that `atl` is actually reachable.

## Public disclosure

We prefer coordinated disclosure. Once a fix is released, we will publish a security advisory describing the issue, the affected versions, and the upgrade path.
