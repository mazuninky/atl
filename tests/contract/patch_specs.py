#!/usr/bin/env python3
"""Patch Jira Platform OpenAPI spec for use with Prism mock server.

Two transforms:
1. Duplicate `/rest/api/3/` paths under `/rest/api/2/` so the spec matches
   both Cloud (v3) and Data Center (v2) code paths in atl. Jira Cloud
   exposes both v2 and v3 at the wire level; DC exposes only v2. Rather
   than renaming v3 out of the spec (which hides the Cloud branches from
   Prism), we keep v3 and add a v2 copy that shares the same schemas.
2. Fix response examples that contain JSON-encoded strings. The official
   Atlassian spec ships ~695 examples as stringified JSON (e.g.
   `"example": "{\"foo\": 1}"`) rather than actual JSON objects. Prism
   returns them verbatim, so clients see a quoted string instead of an
   object. Parse stringified examples into real JSON values where
   possible.
"""

import json
import os

SPEC_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "specs")


def duplicate_jira_paths(spec):
    """Add `/rest/api/2/*` aliases for every `/rest/api/3/*` path.

    The original v3 entries are preserved so Cloud-flavor code (which hits
    `/rest/api/3/...`) is still validated. The v2 copies cover the Data
    Center branch (which hits `/rest/api/2/...`). Both variants share the
    same PathItem object — they are identical at the wire level on Cloud,
    and DC is wire-compatible with v2 for the subset of endpoints we care
    about.
    """
    old_paths = spec.get("paths", {})
    new_paths = dict(old_paths)
    count = 0
    for path, value in old_paths.items():
        if "/rest/api/3/" not in path:
            continue
        v2_path = path.replace("/rest/api/3/", "/rest/api/2/")
        if v2_path in new_paths:
            continue
        new_paths[v2_path] = value
        count += 1
    spec["paths"] = new_paths
    return count


def prefix_paths_with_server_basepath(spec):
    """Prepend the server URL basepath to every path key.

    Prism does not honor the basepath component of `servers[].url` when
    routing requests — it treats spec path keys as root-relative. If the
    spec paths are declared as `/pages` but the real API is served at
    `/wiki/api/v2/pages`, Prism returns 404 for requests sent to
    `/wiki/api/v2/pages`. To fix this we move the basepath into the path
    keys and clear the server URL.
    """
    servers = spec.get("servers") or []
    if not servers:
        return 0
    url = servers[0].get("url", "")
    # Extract basepath after the host, if any.
    # Handle forms like `https://{domain}/wiki/api/v2` and `/wiki/api/v2`.
    import re

    match = re.search(r"^(?:[a-z]+:)?//[^/]+(/.+)$", url)
    if match:
        basepath = match.group(1)
    elif url.startswith("/"):
        basepath = url
    else:
        return 0
    basepath = basepath.rstrip("/")
    if not basepath:
        return 0

    old_paths = spec.get("paths", {})
    new_paths = {p if p.startswith(basepath) else basepath + p: v for p, v in old_paths.items()}
    spec["paths"] = new_paths
    # Clear the servers basepath so Prism routes at root.
    spec["servers"] = [{"url": "/"}]
    return len(old_paths)


def fix_string_examples(obj):
    """Recursively walk the spec and JSON-parse any `example` fields whose
    value is a string that looks like JSON."""
    fixed = 0
    if isinstance(obj, dict):
        for key, value in list(obj.items()):
            if key == "example" and isinstance(value, str):
                stripped = value.strip()
                if stripped and stripped[0] in "{[":
                    try:
                        obj[key] = json.loads(value)
                        fixed += 1
                    except (json.JSONDecodeError, ValueError):
                        pass
            else:
                fixed += fix_string_examples(value)
    elif isinstance(obj, list):
        for item in obj:
            fixed += fix_string_examples(item)
    return fixed


def strip_security_requirements(spec):
    """Remove `security` requirements from every operation.

    Prism validates auth credentials against the spec's security schemes.
    Endpoints like `/app/properties` require OAuth, but atl always uses
    Basic auth. We strip per-operation security so Prism only checks the
    request shape, not the auth method.
    """
    removed = 0
    for path_item in spec.get("paths", {}).values():
        if not isinstance(path_item, dict):
            continue
        for op in path_item.values():
            if isinstance(op, dict) and "security" in op:
                op.pop("security")
                removed += 1
    spec.pop("security", None)
    return removed


def fix_multipart_body_schemas(spec):
    """Rewrite `multipart/form-data` request bodies that are declared as
    `array of MultipartFile`.

    Atlassian's Jira spec describes attachment endpoints with a request body
    of `{ type: array, items: { $ref: MultipartFile } }` — this is a dump of
    the Spring Java DTO rather than the actual multipart form shape. Real
    clients send a plain multipart form with one file field. Rewrite the
    schema to a simple binary file field so Prism validates the real wire
    format.
    """
    fixed = 0
    for path_item in spec.get("paths", {}).values():
        if not isinstance(path_item, dict):
            continue
        for op in path_item.values():
            if not isinstance(op, dict):
                continue
            rb = op.get("requestBody")
            if not isinstance(rb, dict):
                continue
            content = rb.get("content") or {}
            mp = content.get("multipart/form-data")
            if not isinstance(mp, dict):
                continue
            schema = mp.get("schema") or {}
            if schema.get("type") == "array":
                mp["schema"] = {
                    "type": "object",
                    "properties": {
                        "file": {"type": "string", "format": "binary"}
                    },
                    "required": ["file"],
                }
                fixed += 1
    return fixed


def fix_query_array_object_params(spec):
    """Rewrite query parameters declared as `array of object` to plain string.

    The official Jira Agile spec ships several `fields` query params defined
    as `{type: array, items: {type: object}}`, which is nonsensical for a
    query string and Prism rejects any value. Real Jira accepts a comma-
    separated string, so rewrite the schema to match reality.
    """
    fixed = 0
    for path_item in spec.get("paths", {}).values():
        if not isinstance(path_item, dict):
            continue
        for op in path_item.values():
            if not isinstance(op, dict):
                continue
            for param in op.get("parameters", []) or []:
                if not isinstance(param, dict) or param.get("in") != "query":
                    continue
                schema = param.get("schema") or {}
                if (
                    schema.get("type") == "array"
                    and isinstance(schema.get("items"), dict)
                    and schema["items"].get("type") == "object"
                ):
                    param["schema"] = {"type": "string"}
                    fixed += 1
    return fixed


def patch_file(input_name, output_name, duplicate_paths=False, prefix_basepath=False):
    input_path = os.path.join(SPEC_DIR, input_name)
    output_path = os.path.join(SPEC_DIR, output_name)

    with open(input_path) as f:
        spec = json.load(f)

    duplicated = duplicate_jira_paths(spec) if duplicate_paths else 0
    fixed_examples = fix_string_examples(spec)
    fixed_params = fix_query_array_object_params(spec)
    fixed_bodies = fix_multipart_body_schemas(spec)
    stripped_security = strip_security_requirements(spec)
    prefixed = prefix_paths_with_server_basepath(spec) if prefix_basepath else 0

    with open(output_path, "w") as f:
        json.dump(spec, f, separators=(",", ":"))

    print(f"{input_name} -> {output_name}")
    if duplicate_paths:
        print(f"  duplicated {duplicated} paths: /rest/api/3/ -> +/rest/api/2/")
    print(f"  parsed {fixed_examples} stringified examples")
    if fixed_params:
        print(f"  rewrote {fixed_params} query 'array of object' params -> string")
    if fixed_bodies:
        print(f"  rewrote {fixed_bodies} multipart 'array' bodies -> object")
    if stripped_security:
        print(f"  stripped {stripped_security} per-op security requirements")
    if prefixed:
        print(f"  prefixed {prefixed} paths with server basepath")


if __name__ == "__main__":
    patch_file("jira-platform.v3.json", "jira-platform.patched.json", duplicate_paths=True)
    patch_file("jira-software.v3.json", "jira-software.patched.json")
    patch_file("confluence.v3.json", "confluence.patched.json")
    patch_file(
        "confluence-v2.v3.json",
        "confluence-v2.patched.json",
        prefix_basepath=True,
    )
