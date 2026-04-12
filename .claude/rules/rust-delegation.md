---
paths:
  - "src/**/*.rs"
  - "Cargo.toml"
---

## MANDATORY: Rust code changes MUST go through skill + agent

**This is a BLOCKING REQUIREMENT, not a suggestion.** Any edit to `*.rs` files or `Cargo.toml` — no matter how small — MUST follow this workflow. There are NO exceptions for "simple changes", "one-line fixes", dependency bumps, or API migrations.

### Workflow (every time, no exceptions):

1. **Once per session**, invoke the `/rust:cli` skill to load style conventions and agent descriptions into context. Do NOT reload before every agent call — the output stays in context. The purpose is to give the main conversation enough context to pick the right agent and write good prompts.
2. **Pick the right agent** for the task:
   - `rust-cli-writer` — production code (new features, bug fixes, refactors, dependency changes)
   - `rust-cli-test-writer` — tests (new tests, fixing flaky tests, reviewing test coverage, diagnosing "hard to test" code)
   - For bug fixes: `rust-cli-writer` for the fix, then `rust-cli-test-writer` for a regression test
3. **Write a thorough prompt** for the agent. The agent starts with zero context — brief it like a colleague who just walked in. Include:
   - What to change and why (not just "fix the bug" — explain the bug)
   - Specific file paths and line numbers
   - Architecture constraints from CLAUDE.md that apply
   - What NOT to change (scope boundaries)
4. **Verify the agent's result** before reporting to the user:
   - Spot-check key files to confirm the change matches intent
   - Confirm `cargo test` / `cargo clippy` passed (agent should have run them, but verify)
   - Check if the agent skipped anything from the prompt (e.g. requested tests not written, dead code not removed)

### What MUST be delegated:
- ANY edit to `src/**/*.rs` or `Cargo.toml` — including one-line changes, import updates, version bumps, dependency feature flag changes, doc comment edits

### What to handle directly (no delegation needed):
- Reading or exploring Rust code (no file modifications)
- Answering questions about code without making changes

### Self-check before ANY Rust file edit:
> "Am I about to use the Edit/Write tool on a `.rs` file or `Cargo.toml`?"
> If yes → STOP. Load skill (if not yet loaded this session), delegate to agent. No exceptions.
