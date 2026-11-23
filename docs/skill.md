# `atl skill`: the agent skill

The skill for coding agents ([`skill/SKILL.md`](../skill/SKILL.md) and
`skill/references/`) is embedded in the binary, so it always describes the
version it came with. `atl skill` needs no network. `skill/evals/` stays in the
repository.

| Command | What it does |
|---|---|
| `atl skill install [--project] [--force]` | Write the skill and the Claude Code link. |
| `atl skill update [--project] [--force]` | Bring an installed skill to this version. With none installed: says so, exit 0. |
| `atl skill status [--project]` | Where it is, which version, what was changed. |
| `atl skill remove [--project] [--force]` | Remove what atl wrote. |

Each prints one object through the usual output (`-F json`, `--jq`,
`--template` work as everywhere).

## Where

- `~/.agents/skills/atl/`, a directory several agents read.
- When Claude Code is present (`$CLAUDE_CONFIG_DIR`, else `~/.claude`), a
  relative symbolic link to it in its `skills/` directory. Only `install` makes
  the link. On Windows no link is made (`link_state: not_wanted`).
- `--project`: `.agents/skills/atl/` and `.claude/skills/atl` of the current
  directory.

`.atl-skill.json` in the skill directory lists the version and the SHA-256 of
each file atl wrote. While atl writes, it also holds the digests the run is
going to leave (`unfinished`), so a run that was cut short is finished by the
next one, of any version. A file at a path such a run had not written yet is
yours unless it is byte for byte one of atl's.

## What atl does not touch

Refused with exit 5, naming what is in the way:

- **A file of atl's that was edited.** `--force` replaces it.
- **A directory atl did not install**, such as one `npx skills` made. `--force`
  writes atl's files into it and leaves the rest. A file or link where atl
  writes one is replaced only with `--force`, a directory never.
- **A file that was saved while atl ran.** The run stops there; the next one
  goes on.
- **A skill directory that is itself a symbolic link** (a checkout someone works
  on), something that is no directory in place of `.agents` or
  `.agents/skills`, a skill directory of another user, or root in someone
  else's home: never, `--force` or not.
- **Something in place of the Claude Code link**: another link is replaced with
  `--force`, a file or directory never.
- **A manifest of a newer atl, or one that cannot be read** or is damaged (over
  256 files or 160 KiB, among others). A damaged one can be taken over with
  `install --force`; `remove` needs a manifest.

Other files in the directory are yours: they are ignored, and `remove` leaves
the directory if there are any, naming up to three. Commands that write lock
the skill directory and wait at most 10 seconds for another atl, then exit 1.

## Automatic update

After a command, the first run of a newer atl updates the global skill, if atl
wrote it and no file of it was edited or is missing. It prints nothing and does
not change the exit code; a failure is a debug log line. It also finishes an
install or update of its own version that stopped half-way. An edited skill is
left alone; `atl skill status` says so, and `atl self check` adds a `skill`
note.

It does not run under `CI`, with `ATL_NO_SKILL_UPDATE` set to anything but
`0`, as root in someone else's home or with a skill directory of another user,
for `atl skill` and `atl self`, and in debug builds. It never waits for a lock,
never installs a skill, never goes back to an older version, and leaves a
project's skill alone.

`atl self update` updates the skill with the binary (`--no-skill` leaves it).

## `status -F json`

One object with the keys `path`, `installed`, `state`, `version`,
`atl_version`, `edited`, `missing`, `colliding`, `in_the_way`, `unlisted`,
`link` and `link_state`. `colliding` lists files that are not atl's where
`install` or `update` writes one (`--force` replaces them); `in_the_way` lists
what is never replaced, such as a directory; `unlisted` lists every file that
is not atl's, and can hold a directory atl may not read; `missing` includes the
files an interrupted run had not written yet. The state `other_owner` is a skill
directory (or, as root, a home directory) of another user: nothing in it is
read.

## Files

A file that is replaced keeps its mode and owner as far as the process may set
them; a new file has the mode of any new file. Every write goes to a temporary
file beside the target and is renamed over it, bound to the file that was read,
and never through a link.

## Exit codes

0 done; 5 refused; 1 a file could not be read or written, or another atl held
the lock for 10 seconds.
