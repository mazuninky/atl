---
name: Contract mismatch
about: The Atlassian REST API behaves differently from the spec atl was built against (wrong status, missing/extra fields, different shape, undocumented error)
title: 'Contract mismatch: '
labels: bug, contract-mismatch
assignees: ''
---

<!--
Use this template when atl fails (or silently misbehaves) because the live
Atlassian API returns something the OpenAPI spec / our contract tests do not
cover. Common shapes:

  - Spec says 200 with body, server returns 204 No Content (or vice versa).
  - Field documented as required is missing, or its type changed.
  - Cloud and Data Center disagree on the same endpoint.
  - Undocumented error code / payload on a known endpoint.

For unrelated bugs use the regular bug report instead.
-->

### Affected command

<!-- The atl command that surfaced the mismatch. -->

```
atl ...
```

### Affected version

<!-- Paste the output of `atl --version`. -->

```
```

### Atlassian instance

<!-- Contract drift is almost always Cloud-vs-DC or DC-version-specific.
     Be specific — "DC 9.x" is not enough, paste the exact build. -->

- [ ] Atlassian Cloud
- [ ] Data Center / Server
  - Product: <!-- Jira, Confluence, or both -->
  - Version / build: <!-- e.g. Jira Data Center 9.12.5 build 9120005 -->

### Endpoint

<!-- Method + path, exactly as atl calls it. If you are not sure, find it in
     the verbose log (see Raw HTTP below). -->

- Method: <!-- GET / POST / PUT / DELETE -->
- Path:   <!-- e.g. /rest/api/2/issue/{key}/transitions -->
- Auth:   <!-- API token / PAT / basic -->

### What the spec says

<!--
Link to the upstream reference and quote the relevant bit (status code,
field name, type). One link per product if Cloud and DC differ.

  - Cloud: https://developer.atlassian.com/cloud/jira/platform/rest/v2/...
  - DC:    https://docs.atlassian.com/software/jira/docs/api/REST/9.12.0/...
  - Confluence Cloud v2: https://developer.atlassian.com/cloud/confluence/rest/v2/...

If the issue is in our own contract tests (`tests/contract_*.rs`) or the
checked-in OpenAPI fixtures, point at the file + line instead.
-->

### What the server actually returns

<!--
The real response. Status line + headers + body (redacted). The easiest way
to capture this is via the generic passthrough so atl's typed deserialisation
does not hide the shape:

    atl api --method GET --path '/rest/api/2/issue/FOO-1/transitions' -vv

Paste status, response headers, and the body. Keep secrets out.
-->

```http
HTTP/1.1 ...

{ ... }
```

### Reproduction with `atl api`

<!--
A minimal `atl api` invocation that reproduces the mismatch without any of
atl's typed wrappers. This is what we will turn into a contract test.
-->

```
atl api --method ... --path '...' [--input @body.json]
```

### Expected vs actual (in atl terms)

<!-- What atl did, and what it should have done given the real response.
     E.g. "atl errored with `expected JSON body` but the server correctly
     returned 204 — atl should treat 204 as success". -->

- Expected: <!-- ... -->
- Actual:   <!-- ... including any non-zero exit code -->

### Logs

<!--
Re-run with verbose logging so we can see the request/response cycle. Redact
tokens, account IDs, email addresses, and anything else sensitive.

    RUST_LOG=atl=debug,reqwest=debug atl <your command>
    # or
    atl -vv <your command>
-->

```
```

### Additional context

<!-- When did it start? Did a recent Atlassian release change behaviour?
     Marketplace apps installed that might alter the response shape?
     Anything else useful for narrowing the drift. -->
