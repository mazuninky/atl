# Body formats — Atlassian markup vs markdown

This reference covers every body-content format `atl` reads or writes, and
the conversions between them. Read this when deciding what to pass to
`--body-format` / `--input-format`, when a command produced "garbage"
output that turned out to be a format mismatch, or when piping content
between Confluence and Jira.

## The four formats

Atlassian's two products use **different** markup languages. They are
not interchangeable. `atl` exposes four formats across read and write
flows:

| Format | Where | Looks like | Notes |
|---|---|---|---|
| **Confluence storage** | Confluence read/write default | XHTML with custom `<ac:*>`/`<ri:*>` macros | The canonical persistence format. Round-trip safe. |
| **Confluence view** | Confluence read only | Rendered HTML | Read-only — what the page looks like in a browser. Not a write target. |
| **Jira wiki syntax** | Jira create/update/comment default | Plain text with `h1.`, `*bold*`, `{code}`, `\|\|h\|\|`, etc. | Sent as a JSON string to v2 endpoints. Jira renders it server-side. |
| **Markdown** | Input only (write flows) | Standard CommonMark + GFM tables/strikethrough | `atl` converts to the destination's native format before sending. |

ADF (Atlassian Document Format, JSON tree used by Jira Cloud's v3 API) is
**not** currently supported by `atl`'s typed flags. If you need ADF —
mentions, panels, expanding sections, status lozenges — construct the
JSON yourself and POST via `atl api -X POST --service jira ...`.

## Read flow vs write flow

The two flag families are **not** interchangeable, despite similar
names:

- `--body-format <FORMAT>` — applies to **reads**. Tells Atlassian which
  representation to return. Confluence only (`atl c read`,
  `atl c export`).
- `--input-format <FORMAT>` — applies to **writes**. Tells `atl` what
  format your `--body` / `--description` / comment text is in, so it can
  convert before sending. Both Confluence and Jira write commands.

```bash
# READ (Confluence) — choose representation
atl c read 123456                          # storage (default)
atl c read 123456 --body-format storage    # explicit
atl c read 123456 --body-format view       # rendered HTML

# WRITE (Confluence) — declare input format
atl c create --space DEV --title "X" --body @doc.md \
  --input-format markdown                  # convert markdown → storage
atl c create --space DEV --title "Y" --body @page.xhtml \
  --input-format storage                   # send as-is (default)

# WRITE (Jira) — declare input format
atl j create --project PROJ --issue-type Task --summary "X" \
  --description @desc.md --input-format markdown   # convert markdown → wiki
atl j comment PROJ-123 'h2. Heading\n\n*bold*'     # wiki literal (default)
```

Reading a Jira issue returns the description in whatever the API gave
back — typically wiki syntax on v2 endpoints, ADF on v3. There is no
`--body-format` for Jira; if you need a specific representation, hit the
right API version with `atl api`.

## Why the asymmetry exists

Confluence and Jira evolved independently:

- Confluence storage format is HTML-based, hand-editable in a pinch,
  and the only format the Confluence API will persist. Markdown was
  added to `atl` as a writer-side convenience because authoring storage
  XHTML is painful.
- Jira wiki syntax predates ADF and remains the default for v2 endpoints
  (`/rest/api/2/issue`). It's a textual format with one-line markup
  rules. ADF came later for the v3 API and is more powerful but a
  JSON tree. `atl`'s `j create/update/comment` use v2 + wiki by
  default, with a markdown→wiki converter for ergonomics.

The two markdown converters are independent code paths, not a shared
"markdown engine":

- Confluence's `--input-format markdown` runs `comrak::markdown_to_html`
  and sends the resulting XHTML as storage format.
- Jira's `--input-format markdown` parses markdown into a comrak AST and
  walks it to emit Jira wiki syntax (different output, lossy in some
  edge cases).

## Markdown → Confluence storage

Pass `--input-format markdown` to `c create`, `c update`, `c blog
create`, or `c blog update`. The conversion is straight `markdown_to_html`
from comrak with default options — output is XHTML that Confluence
accepts as storage format.

```bash
atl c create --space DEV --title "Design notes" \
  --body @design.md --input-format markdown
atl c update 123456 --title "Design notes" \
  --body @design.md --version 5 --input-format markdown
atl c blog create --space DEV --title "Update" \
  --body @post.md --input-format markdown
```

Round-trip caveats: Confluence storage extends XHTML with macros (info
panels, code blocks with language hints, attachments). Plain markdown
won't produce those — for richer content, write storage XHTML directly
or post-process the markdown output. The conversion is one-way; reading
a page back gives you storage XHTML, not the original markdown.

## Markdown → Jira wiki

Pass `--input-format markdown` to `j create`, `j update`, or `j comment`.
The conversion walks the markdown AST and emits Jira wiki syntax. The
mapping is best-effort and lossy in places:

| Markdown | Jira wiki | Notes |
|---|---|---|
| `# H1` … `###### H6` | `h1. H1` … `h6. H6` | One per level |
| `**bold**` | `*bold*` | |
| `*italic*`, `_italic_` | `_italic_` | Both markdown forms map to wiki underscore |
| `***strong+emph***` | `_*word*_` | Outer-emph wraps inner-strong (CommonMark order) |
| `~~strike~~` | `-strike-` | |
| `` `inline` `` | `{{inline}}` | Falls back to `{noformat}` if content contains `}}` |
| ```` ```lang\n…\n``` ```` | `{code:lang}\n…\n{code}` | Falls back to `{noformat}` if body contains `{code}` |
| ```` ```\n…\n``` ```` | `{code}\n…\n{code}` | No language tag |
| Indented (4-space) code | `{code}\n…\n{code}` | Same path as fenced no-lang |
| `[text](url)` | `[text\|url]` | Pipe separator |
| `[](url)` | `[url]` | Empty text → URL-only form |
| `![alt](url)` | `!url!` | **Alt text dropped** — accept the trade-off or upload as attachment |
| `<https://example.com>` | `[https://example.com\|https://example.com]` | Autolink — text equals URL |
| `- item` / `* item` | `* item` | Bullet list |
| Nested `  - sub` | `** sub` | Depth → repeated `*` |
| `1. item` | `# item` | Numbered list |
| Nested `1.\n   1.` | `## item` | Depth → repeated `#` |
| Mixed `- a\n  1. b` | `* a\n## b` | Inner list type alone determines marker |
| Pipe table | `\|\|h1\|\|h2\|\|\n\|c1\|c2\|` | Header row uses `\|\|`, separator dropped, cell pipes escaped to `\|` |
| `> quote` | `{quote}\n…\n{quote}` | Multi-line collapses to single paragraph (soft breaks → space) |
| `---` (HR) | `----` | Four dashes |
| Hard break (two trailing spaces) | `\\` | Wiki line break |
| Soft break in paragraph | space | Lines join |
| Task list `- [ ] x` | `* x` | No native checkbox in Jira — emitted as plain bullet |
| HTML inline (`<sub>`, `<sup>`) | passthrough | Jira wiki accepts a subset |

Caveats:

- The converter never errors — invalid or unsupported markdown produces
  best-effort output, never a hard fail. Inspect the rendered ticket if
  the input is unusual.
- There is no inverse: `atl j view PROJ-123` returns whatever the API
  gives back (wiki on v2, ADF on v3). It does not convert to markdown.

## ADF (out of scope)

Jira Cloud's v3 API takes Atlassian Document Format — a JSON tree of
typed nodes (`paragraph`, `mediaSingle`, `panel`, `expand`, `mention`).
ADF is more expressive than wiki syntax (panels, mentions, status
lozenges, layouts) but tedious to author by hand.

`atl` does not currently expose `--input-format adf`. If you need ADF:

```bash
# Construct the ADF JSON yourself, then POST via api passthrough
atl api -X POST --service jira rest/api/3/issue \
  --raw-field 'fields={"project":{"key":"PROJ"},"issuetype":{"name":"Task"},"summary":"X","description":<adf-json>}'
```

For most common workloads (headings, lists, code, tables, links) wiki
syntax via the markdown converter is sufficient. Reach for ADF only when
you need a feature wiki cannot express.

## Anti-patterns

**Sending markdown to Jira without `--input-format markdown`**

```bash
# WRONG: Jira interprets the body as wiki syntax — markdown renders as garbage
atl j create --project PROJ --issue-type Task --summary "X" \
  --description "## Steps\n- step 1\n- step 2"
# RIGHT: declare the input as markdown so atl converts to wiki
atl j create --project PROJ --issue-type Task --summary "X" \
  --description "## Steps\n- step 1\n- step 2" --input-format markdown
```

The 201-success response masks the broken render — the only way to
notice is to open the ticket in the UI.

**Sending Confluence storage XHTML to Jira (or wiki to Confluence)**

```bash
# WRONG: storage XHTML is not valid Jira wiki
atl j create --project PROJ --issue-type Task --summary "X" \
  --description '<p>Hello <strong>world</strong></p>'
# WRONG: Jira wiki is not valid Confluence storage
atl c create --space DEV --title "X" --body 'h1. Heading\n*bold*'
```

The two formats look superficially similar but parse completely
differently. Always use the destination's native format, or convert
from markdown.

**Confusing `--body-format` (read) with `--input-format` (write)**

```bash
# WRONG: --body-format is a read flag; create takes --input-format
atl c create --space DEV --title "X" --body @doc.md --body-format markdown
# RIGHT
atl c create --space DEV --title "X" --body @doc.md --input-format markdown
```

**Expecting markdown back from a read**

```bash
# WRONG: there is no markdown body-format — Confluence stores XHTML
atl c read 123456 --body-format markdown
# RIGHT: pick storage (XHTML) or view (rendered HTML)
atl c read 123456 --body-format view
```

If you need markdown out, post-process the storage/view output yourself
(e.g. `pandoc -f html -t markdown`).
