# Command-line syntax

This page documents the notation used throughout `atl`'s help text and documentation when describing command shapes. The conventions follow standard Unix man-page notation.

## Notation

| Notation            | Meaning                                                                                   |
|---------------------|-------------------------------------------------------------------------------------------|
| Plain text          | A literal part of the command you must type as shown.                                     |
| `<value>`           | A placeholder the user must replace with a real value.                                    |
| `[optional]`        | An optional part — the command works whether you include it or not.                      |
| `{a \| b \| c}`     | A required choice — you must pick exactly one of the listed alternatives.                 |
| `...`               | The preceding argument may be repeated.                                                   |
| `<two-word-name>`   | Multi-word placeholders use `dash-case`.                                                  |

These symbols are **metasyntax** — they describe the shape of a command, not characters you type.

## Examples

### Literal text

Fixed keywords appear verbatim:

```
atl jira me
atl confluence space list
```

You type `jira`, `me`, `confluence`, `space`, `list` exactly as shown.

### Placeholders

Angle brackets mark values you must supply:

```
atl jira view <issue-key>
atl confluence read <page-id>
```

In practice:

```sh
atl jira view PROJ-123
atl confluence read 123456
```

### Optional arguments

Square brackets mark parts you may omit:

```
atl jira search <jql> [--limit <n>] [--reverse]
```

All of these are valid invocations:

```sh
atl jira search "project = PROJ AND status = Open"
atl jira search "project = PROJ AND status = Open" --limit 20
atl jira search "project = PROJ AND status = Open" --limit 20 --reverse
```

### Required alternatives

Braces with pipes mark a required choice — you must pick exactly one:

```
atl completions {bash | zsh | fish | powershell | elvish}
```

Valid:

```sh
atl completions zsh
```

Invalid (missing choice):

```sh
atl completions
```

### Repeatable arguments

An ellipsis after a placeholder means the argument may appear more than once:

```
atl api <endpoint> [--header <key:value>]... [--query <key=value>]...
```

Both of these are valid:

```sh
atl api --service jira rest/api/2/myself
atl api --service jira rest/api/2/search \
    --query jql='project = PROJ' \
    --query fields=summary,status \
    --header 'Accept: application/json'
```

### Multi-word placeholders

Placeholders that contain more than one word use `dash-case` for readability:

```
atl jira create --project <project-key> --issue-type <type> --summary <text>
atl confluence create --space <space-key> --title <title> --body <body-or-@file>
```

## Global flags vs command flags

`atl` distinguishes between **global flags**, which apply to every command, and **command flags**, which belong to a specific subcommand.

Global flags may appear anywhere on the line — before or after the subcommand — and are always listed in `atl --help`. The main ones are:

```
atl [-v | -vv | -vvv] [-q] [--no-color] [--no-pager]
     [-F {console | json | toon | toml | csv}]
     [-p <profile>] [--config <path>]
     <command> ...
```

Command-specific flags only appear in the help of that subcommand (`atl jira search --help`) and must be written after the subcommand name.

## Reading the help

Every subcommand prints its own usage line in the standard notation:

```sh
atl jira search --help
```

The first line of the output is the shape; the rest explains each flag. If something in this page is unclear, the per-command help is the authoritative reference.
