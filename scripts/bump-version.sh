#!/usr/bin/env bash
# bump-version.sh — compute next YYYY.WW.BUILD, update Cargo.toml, commit, tag.
#
# Source of truth: git tags `vYYYY.WW.BUILD`.
# Usage:
#   scripts/bump-version.sh [--dry-run] [--force-branch]
#
#   --dry-run       Print the next version and exit. No side effects.
#   --force-branch  Allow running on a branch other than `master`.
#
# Exit codes:
#   0  success
#   1  wrong branch, dirty tree, or bump failure
#   2  usage error

set -euo pipefail

DRY_RUN=0
FORCE_BRANCH=0

usage() {
    cat <<EOF
Usage: $(basename "$0") [--dry-run] [--force-branch]

Computes the next YYYY.WW.BUILD version from git tags, rewrites
Cargo.toml, commits, and creates an annotated tag. Does not push.
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run)      DRY_RUN=1 ;;
        --force-branch) FORCE_BRANCH=1 ;;
        -h|--help)      usage; exit 0 ;;
        *)              echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
    shift
done

REPO_ROOT=$(git rev-parse --show-toplevel)
cd "$REPO_ROOT"

if [ ! -f Cargo.toml ]; then
    echo "error: Cargo.toml not found at $REPO_ROOT" >&2
    exit 1
fi

# Refuse to run on a dirty tree (unless dry-run: reading git state is harmless).
if [ "$DRY_RUN" -eq 0 ]; then
    if ! git diff --quiet \
        || ! git diff --cached --quiet \
        || [ -n "$(git ls-files --others --exclude-standard)" ]; then
        echo "error: working tree is dirty; commit or stash changes first" >&2
        exit 1
    fi

    CURRENT_BRANCH=$(git rev-parse --abbrev-ref HEAD)
    if [ "$CURRENT_BRANCH" != "master" ] && [ "$FORCE_BRANCH" -eq 0 ]; then
        echo "error: releases must be cut from 'master' (currently on '$CURRENT_BRANCH')" >&2
        echo "       pass --force-branch to override" >&2
        exit 1
    fi
fi

# Fetch tags so the BUILD calculation sees every existing release.
if [ "$DRY_RUN" -eq 0 ]; then
    git fetch --tags --quiet
fi

YEAR=$(date -u +%G)
WEEK=$(date -u +%V)

LATEST_TAG=$(git tag -l "v${YEAR}.${WEEK}.*" --sort=-v:refname | head -n 1 || true)
if [ -z "$LATEST_TAG" ]; then
    BUILD=1
else
    LATEST_BUILD=${LATEST_TAG##*.}
    if ! printf '%s' "$LATEST_BUILD" | grep -Eq '^[0-9]+$'; then
        echo "error: could not parse BUILD number from tag '$LATEST_TAG'" >&2
        exit 1
    fi
    BUILD=$((LATEST_BUILD + 1))
fi

VERSION="${YEAR}.${WEEK}.${BUILD}"
TAG="v${VERSION}"

if [ "$DRY_RUN" -eq 1 ]; then
    echo "$VERSION"
    exit 0
fi

# Sanity: the computed tag must not already exist.
if git rev-parse -q --verify "refs/tags/${TAG}" >/dev/null; then
    echo "error: tag ${TAG} already exists" >&2
    exit 1
fi

# Rewrite the [package] version line in Cargo.toml. Only touches the first
# `version = "..."` that appears under `[package]` — never mangles dependency
# version constraints.
TMP=$(mktemp)
trap 'rm -f "$TMP"' EXIT

awk -v new="$VERSION" '
    BEGIN { in_pkg = 0; done = 0 }
    /^\[package\][[:space:]]*$/ { in_pkg = 1; print; next }
    /^\[/ && !/^\[package\][[:space:]]*$/ { in_pkg = 0; print; next }
    {
        if (in_pkg && !done && $0 ~ /^version[[:space:]]*=/) {
            print "version = \"" new "\""
            done = 1
            next
        }
        print
    }
    END {
        if (!done) {
            print "error: no [package] version line found in Cargo.toml" > "/dev/stderr"
            exit 1
        }
    }
' Cargo.toml > "$TMP"

mv "$TMP" Cargo.toml
trap - EXIT

# Refresh Cargo.lock so the root package version entry matches.
cargo check --locked >/dev/null 2>&1 || cargo check >/dev/null

git add Cargo.toml Cargo.lock
git commit -m "release: ${TAG}"
git tag -a "${TAG}" -m "Release ${VERSION}"

echo "Bumped to ${VERSION}"
echo "Created commit $(git rev-parse --short HEAD) and tag ${TAG}"
echo "Push with: git push origin ${CURRENT_BRANCH} && git push origin ${TAG}"
