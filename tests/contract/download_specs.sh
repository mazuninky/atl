#!/usr/bin/env bash
set -euo pipefail

SPEC_DIR="$(cd "$(dirname "$0")" && pwd)/specs"
mkdir -p "$SPEC_DIR"

download() {
    local name="$1" url="$2"
    local dest="$SPEC_DIR/$name"
    if [ -f "$dest" ] && [ -s "$dest" ]; then
        echo "  skip $name (exists)"
        return
    fi
    echo "  downloading $name ..."
    # Download to a temp file first so an interrupted fetch never leaves a
    # partial spec in place that the next run would mistake for a cached
    # download. mktemp inside the same directory keeps the rename atomic on
    # the same filesystem.
    local tmp
    tmp=$(mktemp "$SPEC_DIR/.${name}.XXXXXX")
    trap 'rm -f "$tmp"' EXIT
    if ! curl -fsSL -o "$tmp" "$url"; then
        rm -f "$tmp"
        trap - EXIT
        echo "  FAILED $name" >&2
        return 1
    fi
    mv "$tmp" "$dest"
    trap - EXIT
    echo "  done ($name: $(wc -c < "$dest" | tr -d ' ') bytes)"
}

echo "Downloading OpenAPI specs to $SPEC_DIR"

download "jira-platform.v3.json" \
    "https://developer.atlassian.com/cloud/jira/platform/swagger-v3.v3.json"

download "jira-software.v3.json" \
    "https://developer.atlassian.com/cloud/jira/software/swagger.v3.json"

download "confluence.v3.json" \
    "https://developer.atlassian.com/cloud/confluence/swagger.v3.json"

download "confluence-v2.v3.json" \
    "https://dac-static.atlassian.com/cloud/confluence/rest/v2/swagger.v3.json"

echo "All specs downloaded."
