#!/bin/sh
#
# Installer for the atl CLI tool.
# https://github.com/mazuninky/atl
#
# Usage:
#   curl -sSfL https://raw.githubusercontent.com/mazuninky/atl/master/scripts/install.sh | sh
#
# Options:
#   --version VERSION    Install a specific version (without the "v" prefix).
#                        Defaults to the latest release.
#   --install-dir DIR    Directory to install the binary into.
#                        Defaults to /usr/local/bin if writable, otherwise ~/.local/bin.
#
# Examples:
#   # Install the latest release
#   curl -sSfL https://raw.githubusercontent.com/mazuninky/atl/master/scripts/install.sh | sh
#
#   # Install a specific version
#   curl -sSfL https://raw.githubusercontent.com/mazuninky/atl/master/scripts/install.sh | sh -s -- --version 2026.15.1
#
#   # Install to a custom directory
#   curl -sSfL https://raw.githubusercontent.com/mazuninky/atl/master/scripts/install.sh | sh -s -- --install-dir ~/bin

set -eu

REPO="mazuninky/atl"
GITHUB="https://github.com"
BINARY_NAME="atl"

# --- helpers ----------------------------------------------------------------

log() {
    printf '%s\n' "$@"
}

err() {
    printf 'Error: %s\n' "$1" >&2
    exit 1
}

need_cmd() {
    if ! command -v "$1" > /dev/null 2>&1; then
        err "required command not found: $1"
    fi
}

# --- argument parsing -------------------------------------------------------

VERSION=""
INSTALL_DIR=""

while [ $# -gt 0 ]; do
    case "$1" in
        --version)
            [ $# -ge 2 ] || err "--version requires a value"
            VERSION="${2#v}"
            shift 2
            ;;
        --install-dir)
            [ $# -ge 2 ] || err "--install-dir requires a value"
            INSTALL_DIR="$2"
            shift 2
            ;;
        -h|--help)
            cat <<HELPEOF
Installer for the atl CLI tool.
https://github.com/mazuninky/atl

Usage:
  curl -sSfL https://raw.githubusercontent.com/mazuninky/atl/master/scripts/install.sh | sh

Options:
  --version VERSION    Install a specific version (without the "v" prefix).
                       Defaults to the latest release.
  --install-dir DIR    Directory to install the binary into.
                       Defaults to /usr/local/bin if writable, otherwise ~/.local/bin.

Examples:
  # Install the latest release
  curl -sSfL .../install.sh | sh

  # Install a specific version
  curl -sSfL .../install.sh | sh -s -- --version 2026.15.1

  # Install to a custom directory
  curl -sSfL .../install.sh | sh -s -- --install-dir ~/bin
HELPEOF
            exit 0
            ;;
        *)
            err "unknown option: $1"
            ;;
    esac
done

# --- platform detection -----------------------------------------------------

detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Linux)
            case "$ARCH" in
                x86_64)  TARGET="x86_64-unknown-linux-gnu" ;;
                *)       err "unsupported Linux architecture: $ARCH. Only x86_64 is supported." ;;
            esac
            ;;
        Darwin)
            case "$ARCH" in
                arm64)   TARGET="aarch64-apple-darwin" ;;
                *)       err "unsupported macOS architecture: $ARCH. Only arm64 (Apple Silicon) is supported." ;;
            esac
            ;;
        MINGW*|MSYS*|CYGWIN*|Windows_NT)
            err "automatic installation is not supported on Windows. Please download the binary manually from: ${GITHUB}/${REPO}/releases/latest"
            ;;
        *)
            err "unsupported operating system: $OS"
            ;;
    esac
}

# --- http client selection --------------------------------------------------

download() {
    url="$1"
    output="$2"

    if command -v curl > /dev/null 2>&1; then
        curl -sSfL -o "$output" "$url"
    elif command -v wget > /dev/null 2>&1; then
        wget -q -O "$output" "$url"
    else
        err "either curl or wget is required to download files"
    fi
}

# Fetch a URL and print the response body to stdout.
download_to_stdout() {
    url="$1"

    if command -v curl > /dev/null 2>&1; then
        curl -sSfL "$url"
    elif command -v wget > /dev/null 2>&1; then
        wget -q -O - "$url"
    else
        err "either curl or wget is required to download files"
    fi
}

# --- version resolution -----------------------------------------------------

resolve_version() {
    if [ -n "$VERSION" ]; then
        return
    fi

    log "Resolving latest version..."

    # The GitHub releases/latest endpoint redirects to the actual tag.
    # We follow the redirect and extract the tag from the final URL.
    if command -v curl > /dev/null 2>&1; then
        REDIRECT_URL="$(curl -sSfL -o /dev/null -w '%{url_effective}' "${GITHUB}/${REPO}/releases/latest")"
    elif command -v wget > /dev/null 2>&1; then
        # wget prints the final URL in its server response output
        REDIRECT_URL="$(wget --spider -S -q -O /dev/null "${GITHUB}/${REPO}/releases/latest" 2>&1 | grep -i 'Location:' | tail -1 | sed 's/.*Location: *//' | tr -d '\r')"
    else
        err "either curl or wget is required"
    fi

    # Extract version from URL: .../releases/tag/vYYYY.WW.BUILD -> YYYY.WW.BUILD
    TAG="$(printf '%s' "$REDIRECT_URL" | sed 's|.*/tag/||')"
    [ -n "$TAG" ] || err "failed to resolve the latest release version"

    # Strip the leading "v" if present.
    VERSION="$(printf '%s' "$TAG" | sed 's/^v//')"
    [ -n "$VERSION" ] || err "failed to parse version from tag: $TAG"
}

# --- checksum verification --------------------------------------------------

verify_checksum() {
    archive_path="$1"
    checksum_path="$2"

    expected="$(cut -d ' ' -f 1 < "$checksum_path")"
    [ -n "$expected" ] || err "checksum file is empty or malformed"

    if command -v sha256sum > /dev/null 2>&1; then
        actual="$(sha256sum "$archive_path" | cut -d ' ' -f 1)"
    elif command -v shasum > /dev/null 2>&1; then
        actual="$(shasum -a 256 "$archive_path" | cut -d ' ' -f 1)"
    else
        err "neither sha256sum nor shasum found; cannot verify archive integrity"
    fi

    if [ "$expected" != "$actual" ]; then
        err "checksum mismatch (expected: $expected, got: $actual)"
    fi

    log "Checksum verified."
}

# --- install directory selection --------------------------------------------

resolve_install_dir() {
    if [ -n "$INSTALL_DIR" ]; then
        return
    fi

    if [ -d /usr/local/bin ] && [ -w /usr/local/bin ]; then
        INSTALL_DIR="/usr/local/bin"
    else
        INSTALL_DIR="${HOME}/.local/bin"
    fi
}

# --- main -------------------------------------------------------------------

main() {
    detect_platform
    resolve_version
    resolve_install_dir

    ARCHIVE_NAME="${BINARY_NAME}-${VERSION}-${TARGET}.tar.gz"
    CHECKSUM_NAME="${ARCHIVE_NAME}.sha256"
    DOWNLOAD_URL="${GITHUB}/${REPO}/releases/download/v${VERSION}/${ARCHIVE_NAME}"
    CHECKSUM_URL="${GITHUB}/${REPO}/releases/download/v${VERSION}/${CHECKSUM_NAME}"

    TMPDIR_INSTALL="$(mktemp -d)"
    # Clean up on exit regardless of success or failure.
    trap 'rm -rf "$TMPDIR_INSTALL"' EXIT

    log "Installing atl v${VERSION} for ${TARGET}..."
    log ""
    log "  Archive:     ${ARCHIVE_NAME}"
    log "  Install dir: ${INSTALL_DIR}"
    log ""

    log "Downloading ${ARCHIVE_NAME}..."
    download "$DOWNLOAD_URL" "${TMPDIR_INSTALL}/${ARCHIVE_NAME}"

    log "Downloading ${CHECKSUM_NAME}..."
    download "$CHECKSUM_URL" "${TMPDIR_INSTALL}/${CHECKSUM_NAME}"

    verify_checksum "${TMPDIR_INSTALL}/${ARCHIVE_NAME}" "${TMPDIR_INSTALL}/${CHECKSUM_NAME}"

    log "Extracting..."
    tar -xzf "${TMPDIR_INSTALL}/${ARCHIVE_NAME}" -C "$TMPDIR_INSTALL"

    # The archive contains a directory: atl-VERSION-TARGET/
    EXTRACTED_DIR="${TMPDIR_INSTALL}/${BINARY_NAME}-${VERSION}-${TARGET}"
    [ -f "${EXTRACTED_DIR}/${BINARY_NAME}" ] || err "binary not found in archive at ${EXTRACTED_DIR}/${BINARY_NAME}"

    mkdir -p "$INSTALL_DIR"
    cp "${EXTRACTED_DIR}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
    chmod +x "${INSTALL_DIR}/${BINARY_NAME}"

    # Ad-hoc codesign on macOS so the binary can access the login keychain
    # without triggering a password prompt on every rebuild / reinstall.
    if [ "$(uname -s)" = "Darwin" ] && command -v codesign >/dev/null 2>&1; then
        codesign -s - -f "${INSTALL_DIR}/${BINARY_NAME}" 2>/dev/null || true
    fi

    log ""
    log "atl v${VERSION} installed to ${INSTALL_DIR}/${BINARY_NAME}"

    # Check if the install directory is in PATH.
    case ":${PATH}:" in
        *":${INSTALL_DIR}:"*)
            ;;
        *)
            log ""
            log "Warning: ${INSTALL_DIR} is not in your PATH."
            log "Add it by running:"
            log ""
            log "  export PATH=\"${INSTALL_DIR}:\$PATH\""
            log ""
            log "You may want to add this line to your shell profile (~/.bashrc, ~/.zshrc, etc.)."
            ;;
    esac

    # Verify the installed binary works.
    if VERSION_OUT="$("${INSTALL_DIR}/${BINARY_NAME}" --version 2>&1)"; then
        log ""
        log "Verified: ${VERSION_OUT}"
    else
        err "installed binary failed to run: ${VERSION_OUT:-unknown error}"
    fi
}

main
