---
paths:
  - "src/**/*.rs"
  - "Cargo.toml"
---

When the user asks to write, modify, fix, or refactor Rust CLI code, first invoke the `/rust:cli` skill to load style conventions, then delegate the implementation to the `rust-cli-writer` agent (Agent tool with `subagent_type: "rust-cli-writer"`). Include the skill's conventions in the agent prompt so it follows project patterns.

Delegate to agent for:
- Creating new CLI components or commands
- Adding/modifying subcommands
- Implementing features, fixing bugs, refactoring

Do NOT delegate (handle directly):
- Reading or exploring Rust code
- Answering questions about code without changes
