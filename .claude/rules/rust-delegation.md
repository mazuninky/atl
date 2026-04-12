---
paths:
  - "src/**/*.rs"
  - "Cargo.toml"
---

## MANDATORY: Rust code changes MUST go through skill + agent

**This is a BLOCKING REQUIREMENT, not a suggestion.** Any edit to `*.rs` files or `Cargo.toml` — no matter how small — MUST follow this workflow. There are NO exceptions for "simple changes", "one-line fixes", dependency bumps, or API migrations.

### Workflow (every time, no exceptions):

1. **Once per session**, invoke the `/rust:cli` skill to load style conventions. Do NOT reload before every agent call — the output stays in context.
2. **Then**, delegate the implementation to the `rust-cli-writer` agent (`Agent` tool with `subagent_type: "rust-cli-writer"`). Include full context (what to change, why, which files, relevant API details) in the agent prompt.

### What MUST be delegated:
- ANY edit to `src/**/*.rs` or `Cargo.toml` — including one-line changes, import updates, version bumps, dependency feature flag changes, doc comment edits

### What to handle directly (no delegation needed):
- Reading or exploring Rust code (no file modifications)
- Answering questions about code without making changes

### Self-check before ANY Rust file edit:
> "Am I about to use the Edit/Write tool on a `.rs` file or `Cargo.toml`?"
> If yes → STOP. Load skill (if not yet loaded this session), delegate to agent. No exceptions.
