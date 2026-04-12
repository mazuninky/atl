#!/usr/bin/env bash
#
# Generate docs via the hidden `atl generate-docs` subcommand and copy the
# committed reference snapshot into docs/reference/. CI runs this and then
# asserts `git diff --exit-code docs/reference/` to guard docs freshness.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="target/_docs_tmp"

cargo build --quiet
rm -rf "$OUT_DIR"
./target/debug/atl generate-docs --output-dir "$OUT_DIR"

rm -rf docs/reference
mkdir -p docs/reference
cp -f "$OUT_DIR"/reference/*.md docs/reference/
