# Body formats — markdown is the default

This reference covers every body-content format `atl` reads or writes,
the conversions between them, and the MyST-style directive grammar that
makes Confluence panels and Jira macros expressible in plain markdown.
Read this when deciding what to pass to `--body-format` /
`--input-format`, when a command produced "garbage" output that turned
out to be a format mismatch, or when piping content between Confluence
and Jira.

## TL;DR

- **Markdown is the default**, on both read and write, for both Confluence and Jira.
- `atl c read 12345` returns markdown. `atl j view PROJ-1` returns an issue with markdown in `.fields.description` and each comment body.
- `atl c create --body @file.md` and `atl j create --description @file.md` both expect markdown by default.
- Confluence panels (`<ac:structured-macro ac:name="info">…`) and Jira macros (`{info}…{info}`) round-trip through `:::info … :::` block directives; inline things like status lozenges, mentions, emoticons are inline directives (`:status[DONE]{color=green}`).
- For raw payloads use `--body-format storage` / `wiki` / `adf` on read, or `--input-format storage` / `wiki` / `adf` on write.

## The four formats

| Format | Where | Looks like |
|---|---|---|
| **Markdown** | Default for both services on both read and write | CommonMark + GFM tables/strike + MyST directives (`:::info`, `:status[…]{…}`) |
| **Confluence storage** | Confluence raw payload | XHTML with `<ac:*>` / `<ri:*>` macros — Confluence's canonical persistence format |
| **Jira wiki syntax** | Jira raw payload | Plain text with `h1.`, `*bold*`, `{code}`, `{info}…{info}`, `\|\|h\|\|`, etc. |
| **ADF (Atlassian Document Format)** | Cloud-native JSON tree | `{"type":"doc","version":1,"content":[…]}` — used by Confluence v2 and Jira Cloud v3 APIs |

`atl` ships **six** converters between markdown and the three native
formats — both directions for each — so a `read | edit | update` loop
through markdown is lossless for the documented constructs and
gracefully degrades for the rest.

## Read vs write

The two flag families are **not** interchangeable, despite similar names:

- `--body-format <FORMAT>` — applies to **reads**. Tells `atl` what
  format to return on stdout. Available on `atl c read`, `atl c export`,
  `atl c blog read`, `atl j view`, `atl j comments`, `atl j comment-get`.
- `--input-format <FORMAT>` — applies to **writes**. Declares the format
  of the body / description / comment text being sent. Available on
  Confluence page/blog `create` / `update` and on every Jira
  `create` / `update` / `comment` command. **Not** available on
  Confluence comment commands (`atl c comment …`, `atl c inline-comment
  …`) — those still expect raw storage XHTML.

Available values:

| Service | `--body-format` (read) | `--input-format` (write) |
|---|---|---|
| Confluence | `markdown` (default), `storage`, `view`, `adf` | `markdown` (default), `storage`, `adf` |
| Jira | `markdown` (default), `wiki`, `adf` | `markdown` (default), `wiki`, `adf` |

`view` is rendered HTML — read-only; not a write format.

`adf` for **Jira** is Cloud-only and routes through the v3 API. On Data
Center / Server, `--body-format adf` and `--input-format adf` for Jira
fail fast with `Error::Config` because v3 doesn't exist there. ADF for
**Confluence** works on both Cloud and Data Center via v2's
`body-format=atlas_doc_format` query.

## Directive grammar

Markdown alone can't express Confluence info panels, expanding sections,
TOCs, status lozenges, mentions, or emoticons. `atl` extends markdown
with two MyST-inspired forms:

### Block directives (fenced)

```
:::info
This is an info panel.
:::

:::warning title="Heads up"
Multi-paragraph warning body.

Still inside the panel.
:::

:::expand title="Click to expand"
Hidden content.

:::info
Nested info inside an expand.
:::

More expand body.
:::

:::toc maxLevel=3
:::
```

- Open: `:::name` at column 0, optionally followed by `key="value"` pairs.
- Close: `:::` on its own line.
- Nesting: depth-counted by the lexer; just keep using `:::` everywhere — the parser pairs opens with closes via a stack.
- Self-closing block directives (`toc`) — open + immediate close.
- Indented `:::` (`    :::info`) is a CommonMark code block, **not** a directive.
- `:::` inside a fenced code block is just text.

### Inline directives (role)

```markdown
release :status[DONE]{color=green} ready
notify :mention[@john]{accountId=abc123}
:emoticon{name=warning} careful
see :link[Page Title]{pageId=12345}
inline image :image{src="https://x/y.png" alt="diagram"}
```

- `:name` — identifier, must be preceded by start-of-string or non-alphanumeric (so `https://` doesn't trigger).
- Optional `[content]` — text body for content-bearing directives.
- Optional `{key=value key2="value with spaces"}` — attributes.
- Self-closing form (no `[…]`): `:name{…}` or just `:name`.

### Supported names

| Block | Confluence | Jira |
|---|---|---|
| `:::info`, `:::warning`, `:::note`, `:::tip` | `<ac:structured-macro ac:name="info">…` (and ADF `panel` with matching `panelType`) | `{info}…{info}` (and ADF `panel`) |
| `:::expand title="…"` | `<ac:structured-macro ac:name="expand">` / ADF `expand` | No native equivalent — falls back to `*Title*\n\nbody` |
| `:::toc [maxLevel=N]` | `<ac:structured-macro ac:name="toc">` / ADF `extension` | `{toc}` macro |

| Inline | Confluence | Jira |
|---|---|---|
| `:status[TEXT]{color=...}` | `<ac:structured-macro ac:name="status">…` / ADF `status` | `{status:colour=...&#124;title=...}` / ADF `status` |
| `:emoticon{name=...}` | `<ac:emoticon ac:name="..."/>` / ADF `emoji` | `(!)` / `(/)` / `(x)` / `(i)` / `(?)` shortcuts |
| `:mention[@name]{accountId=...}` | `<ac:link><ri:user/>` / ADF `mention` | `[~accountid:...]` / ADF `mention` |
| `:link[Title]{pageId=N &#124; url=...}` | `<ac:link><ri:page/>` / ADF `inlineCard` | `[Title&#124;url]` |
| `:image{src="..." alt="..."}` | `<ac:image>` / ADF `mediaSingle` | `!url&#124;alt=...!` |

Unknown directive names pass through as text — round-trip safe.

### Strip directives on read

`--no-directives` (on `c read`, `c export`, `c blog read`, `j view`,
`j comments`, `j comment-get`) flattens panels and inline directives to
their content text. Useful when you just want the body text without
macro markers:

```bash
atl c read 12345 --no-directives                 # markdown without :::info wrappers
atl c read 12345 --body-format markdown          # markdown WITH :::info wrappers (default)
```

## Recipes

### Read a page, edit, push back

```bash
atl c info 12345                                 # find current title + version
atl c read 12345 > /tmp/p.md                     # markdown by default
$EDITOR /tmp/p.md
atl c update 12345 --title "..." --version 6 --body @/tmp/p.md
```

Round-trip is lossless for headings, lists, code blocks, tables, links,
images, panels, expand sections, TOC, status badges, mentions, and
emoticons. Lossy for: media with explicit dimensions/layout, custom
ADF marks (textColor, custom subsup), and any unknown structured macro.

### CI-style search-and-replace

```bash
atl c read 12345 \
  | sed 's/old-name/new-name/g' \
  | atl c update 12345 --title "..." --version 7 --body -
```

`sed` works on markdown the way you'd expect.

### Pipe Confluence to Jira

```bash
atl c read 12345 \
  | atl j comment PROJ-100 --body -
```

Markdown is the lingua franca — both services accept it on input by
default. The Confluence storage XHTML and Jira wiki syntax are different
markup languages that don't interoperate; markdown bridges them.

### Bypass conversion for legacy automation

```bash
atl c read 12345 --body-format storage > raw.xhtml
atl c update 12345 --title "..." --version 6 --body @raw.xhtml --input-format storage

atl j view PROJ-1 --body-format wiki | grep "h2\."
atl j create --project PROJ --issue-type Task --summary "X" \
  --description '{info}critical{info}' --input-format wiki
```

### Send raw ADF (power user)

```bash
atl c create --space DEV --title "X" --body @page.json --input-format adf
atl j create --project PROJ --issue-type Task --summary "X" \
  --description @desc.json --input-format adf
```

ADF must be a complete document (`{"type":"doc","version":1,"content":[…]}`).
For Jira, this requires Cloud (uses v3 API).

## Lossy mappings

The converters are best-effort. The following are documented losses:

- **Markdown → Jira wiki** — alt text on `![alt](url)` is dropped (Jira `!url!` has no alt slot). Task list checkboxes (`- [ ] x`) become plain bullets. Triple-emphasis (`***word***`) emits `_*word*_` per CommonMark order.
- **Markdown → ADF** — inline `:image{…}` is dropped because ADF `mediaSingle` is block-level only (use a block-level image syntax instead). `:link{pageId=N}` synthesizes a `pageId:N` placeholder URL — humans inspecting the JSON see the synthesis.
- **ADF → markdown** — `panel:error` collapses to `:::warning` (no `:::error` directive). `mediaSingle` with custom layout/dimensions emits a basic `![alt](url)` losing the layout info. `extension` nodes other than the TOC macro fall through as `<!-- adf:unknown … -->` JSON comments.
- **Confluence storage → markdown** — Unknown `<ac:structured-macro>` names pass through as raw HTML so the round-trip stays lossless. `<u>` becomes `<u>` HTML in markdown (no native CommonMark underline).
- **Jira wiki → markdown** — Citation `??text??`, sub `~text~`, sup `^text^` emit as raw HTML. Unknown `(parens)` aren't matched as emoticons; only the canonical 10-token set (`(!)`, `(?)`, `(/)`, `(x)`, `(i)`, `(*)`, `(y)`, `(n)`, `(on)`, `(off)`).

## Why six converters

Confluence and Jira evolved independently. Confluence's persistence format is XHTML "storage" (with `ac:` macros); Jira's is wiki text on v2 endpoints and ADF on v3. Markdown is no party's native format. Rather than make users learn two markup languages, `atl` makes markdown the universal user-facing format and ships converters in both directions for each native format:

- `markdown ↔ Confluence storage XHTML`
- `markdown ↔ ADF JSON` (used by both Confluence v2 and Jira Cloud v3)
- `markdown ↔ Jira wiki`

The converters live in `src/cli/commands/converters/`. They share the directive registry in `src/cli/commands/directives.rs` so directive grammar parses once and renders consistently across all three native targets.

## Anti-patterns

**Sending storage XHTML to Jira (or wiki to Confluence)**

```bash
# WRONG: storage XHTML is not valid Jira wiki
atl j create --project PROJ --issue-type Task --summary "X" \
  --description '<p>Hello</p>' --input-format wiki

# WRONG: Jira wiki is not valid Confluence storage
atl c create --space DEV --title "X" --body 'h1. Heading' --input-format storage
```

The two raw formats look superficially similar but parse completely
differently. Use markdown (the default) and let `atl` convert.

**Confusing `--body-format` (read) with `--input-format` (write)**

```bash
# WRONG: --body-format is a read flag
atl c create --space DEV --title "X" --body @doc.md --body-format markdown

# RIGHT: markdown is the default — no flag needed
atl c create --space DEV --title "X" --body @doc.md

# RIGHT (explicit): --input-format on writes
atl c create --space DEV --title "X" --body @doc.md --input-format markdown
```

**`--input-format adf` on Jira Data Center**

```bash
# FAILS: ADF requires Cloud v3
atl j create --project PROJ --issue-type Task --summary "X" \
  --description @body.json --input-format adf
# Error: ADF input is not supported on Data Center / Server (v3 API not available)
```

**Indented directive fence**

```markdown
   :::info
   body
   :::
```

Indented `:::` (4+ spaces) is a CommonMark code block, NOT a directive.
Keep fences at column 0.

**Nested fence with mismatched name**

```markdown
:::expand title="X"
:::info
body
:::
```

The first `:::` after `body` closes `:::info`; the second one would be
needed to close `:::expand`. The lexer pairs by depth, not by name —
use one `:::` per level you opened.
