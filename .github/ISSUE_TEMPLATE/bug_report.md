---
name: Bug report
about: Report a bug or unexpected behaviour in atl
title: ''
labels: bug
assignees: ''
---

### Describe the bug

<!-- A clear and concise description of what the bug is. -->

### Affected version

<!-- Paste the output of `atl --version` below. -->

```
```

### Atlassian instance

<!-- Tick the one that applies and note the product + version. -->

- [ ] Atlassian Cloud
- [ ] Data Center / Server
  - Product: <!-- Jira, Confluence, or both -->
  - Version: <!-- e.g. Jira Data Center 9.12.5 -->

### Steps to reproduce

1. Run `atl ...`
2. Observe `...`

### Expected behaviour

<!-- What you expected to happen. -->

### Actual behaviour

<!-- What actually happened. Include any error messages and non-zero exit codes. -->

### Logs

<!--
Re-run the failing command with verbose logging and paste the relevant
output below. Redact tokens, account IDs, or anything sensitive.

    RUST_LOG=atl=debug atl <your command>
    # or
    atl -vv <your command>
-->

```
```

### Additional context

<!-- OS, shell, how you installed atl (release tarball, `cargo install`, `self update`), profile config shape (without secrets), anything else. -->
