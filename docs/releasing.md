# Releasing atl

`atl` uses calendar-based versioning: **`YYYY.WW.BUILD`**, where `YYYY` is the UTC year, `WW` is the ISO week number (01–53), and `BUILD` is a monotonic counter within that week starting at 1. The source of truth is git tags of the form `vYYYY.WW.BUILD` (e.g. `v2026.15.1`).

Releases are fully automated: pushing a matching tag triggers the release workflow, which cross-compiles for Linux / macOS / Windows, uploads the archives plus SHA-256 sums to a new GitHub Release, and generates release notes from merged pull requests.

This document is the **operator checklist** for cutting a release. It does not describe internal workflow details — for those, read [`.github/workflows/release.yml`](../.github/workflows/release.yml) directly.

## Prerequisites

Before starting:

- Local clone is on branch `master`, up to date with `origin/master`, and clean.
- You can push tags to `origin` (`git push origin <tag>`).
- You have permission to create releases on `mazuninky/atl`.

## Pre-release review (at least a few hours before)

1. Confirm the last CI run on `master` is green:
   ```sh
   gh run list --branch master --workflow ci.yml --limit 5
   ```
2. Skim merged PRs since the previous release:
   ```sh
   git log --oneline $(git describe --tags --abbrev=0)..HEAD
   ```
   If anything looks risky, either back it out or postpone the release.
3. Verify the release notes are going to read well — GitHub generates them from PR titles, so any PRs merged with poor titles are the ones to fix **now** by editing the PR title and retitling the merge commit is not possible post-merge, but you can still fix the PR title so the auto-notes pick up the corrected version.

## Cut the release

1. **Bump the version.** From a clean `master`:
   ```sh
   ./scripts/bump-version.sh
   ```
   The script:
   - Computes the next `YYYY.WW.BUILD` from the latest `vYYYY.WW.*` tag.
   - Rewrites `version = "…"` in the `[package]` section of `Cargo.toml`.
   - Runs `cargo check` so `Cargo.lock` picks up the new version.
   - Commits as `release: vYYYY.WW.BUILD` and creates an annotated tag `vYYYY.WW.BUILD`.

   Dry-run the next version without touching anything:
   ```sh
   ./scripts/bump-version.sh --dry-run
   ```

2. **Push the commit and the tag.** The script prints the exact commands:
   ```sh
   git push origin master
   git push origin vYYYY.WW.BUILD
   ```

3. **Watch the release workflow.** Pushing the tag triggers `.github/workflows/release.yml`:
   ```sh
   gh run watch --exit-status
   ```
   The workflow will:
   - Validate that the pushed tag matches `vYYYY.WW.BUILD` format **and** matches the `version` in `Cargo.toml`. A mismatch fails fast with a clear error.
   - Cross-compile three targets in parallel: `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`.
   - Produce `atl-<version>-<target>.{tar.gz,zip}` plus a sibling `*.sha256`.
   - Create a GitHub Release with `generate_release_notes: true` and attach every archive.

4. **Verify the release.**
   ```sh
   gh release view vYYYY.WW.BUILD
   ```
   Check that all three archives and their `.sha256` companions are attached, and that the generated notes list the PRs you expect.

5. **Smoke-test self-update.** From an older installed binary:
   ```sh
   atl self check
   atl self update
   atl --version   # should now print YYYY.WW.BUILD
   ```

## If something goes wrong

The release pipeline is designed to be re-runnable. If the workflow fails partway through, fix the underlying issue and re-tag rather than patching the broken release.

### Workflow failed before the GitHub Release was created

The tag exists but no release is attached. You have two options:

- **Re-run the failed job** if the failure was transient (runner timeout, flaky download):
  ```sh
  gh run rerun <run-id> --failed
  ```
- **Delete the tag and start over** if the failure is fixable in code:
  ```sh
  git push --delete origin vYYYY.WW.BUILD
  git tag -d vYYYY.WW.BUILD
  # fix the bug, land it on master, then rerun scripts/bump-version.sh
  ```
  The next bump will reuse the same `YYYY.WW.BUILD` number because nothing has consumed it yet.

### GitHub Release exists but is wrong (missing asset, bad notes, wrong commit)

1. **Delete the release and the tag** on the remote:
   ```sh
   gh release delete vYYYY.WW.BUILD --cleanup-tag --yes
   ```
   `--cleanup-tag` deletes the tag on the remote too. Also delete locally:
   ```sh
   git tag -d vYYYY.WW.BUILD
   ```
2. Land the fix on `master`, rerun the bump script, and re-tag.

### Version in Cargo.toml disagrees with the tag

This is what `verify-version` catches. It means someone created a tag without running `scripts/bump-version.sh`, or edited `Cargo.toml` manually after the bump. Always cut releases through the script — do not hand-craft tags.

## What *not* to do

- **Do not hand-edit `Cargo.toml` to bump the version.** The script is the only supported path.
- **Do not force-push to a release tag.** Tags are immutable from users' perspective; delete and re-create instead.
- **Do not skip the pre-release CI check.** The release workflow does build-test as part of the cross-compile, but a broken test on `master` means a broken release.
- **Do not cut releases from a branch other than `master`.** The bump script refuses this by default; `--force-branch` exists for emergencies only.
