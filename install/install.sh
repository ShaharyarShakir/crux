#!/usr/bin/env bash
# Crux Installation Script
# https://github.com/ShaharyarShakir/crux
#
# Recommended usage:
#   curl -fsSL https://raw.githubusercontent.com/ShaharyarShakir/crux/main/install/install.sh -o install-crux.sh
#   less install-crux.sh # Inspect before executing
#   bash install-crux.sh
#
# Or direct execution:
#   curl -fsSL https://raw.githubusercontent.com/ShaharyarShakir/crux/main/install/install.sh | bash

set -euo pipefail

CRUX_REPO="ShaharyarShakir/crux"
DEFAULT_VERSION="v0.3.0"
VERSION="${CRUX_VERSION:-$DEFAULT_VERSION}"
INSTALL_DIR="${CRUX_INSTALL_DIR:-$HOME/.local/bin}"

# Parse optional command-line flags
while [[ $# -gt 0 ]]; do
    case "$1" in
        --version|-v)
            [[ $# -ge 2 ]] || { echo "Error: '$1' requires a value" >&2; exit 2; }
            VERSION="$2"
            shift 2
            ;;
        --bin-dir|-d)
            [[ $# -ge 2 ]] || { echo "Error: '$1' requires a value" >&2; exit 2; }
            INSTALL_DIR="$2"
            shift 2
            ;;
        --help|-h)
            cat << 'EOF'
Crux Installer

Usage:
    install.sh [options]

Options:
    -v, --version <VERSION>  Version tag to install (default: v0.3.0)
    -d, --bin-dir <DIR>      Directory to install crux binary (default: ~/.local/bin)
    -h, --help               Display this help text
EOF
            exit 0
            ;;
        *)
            echo "Error: Unknown argument '$1'" >&2
            echo "Run 'install.sh --help' for usage." >&2
            exit 2
            ;;
    esac
done

# Ensure version has 'v' prefix
if [[ ! "$VERSION" =~ ^v ]]; then
    VERSION="v${VERSION}"
fi

# The version becomes part of URLs and file names: accept only a strict
# release-tag form so it can never inject path separators or shell syntax.
if [[ ! "$VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]; then
    echo "Error: invalid version '$VERSION' (expected a release tag such as v0.3.0)" >&2
    exit 2
fi

echo "=== Crux Installer ($VERSION) ==="

# 1. Detect Operating System and Architecture
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Linux)
        OS_TAG="unknown-linux-gnu"
        ;;
    *)
        echo "Error: Crux currently supports Linux only. Detected OS: $OS" >&2
        echo "macOS and Windows support is planned for future releases." >&2
        exit 1
        ;;
esac

case "$ARCH" in
    x86_64|amd64)
        ARCH_TAG="x86_64"
        ;;
    *)
        echo "Error: Unsupported architecture '$ARCH'. Prebuilt binaries are currently available for x86_64 only." >&2
        echo "You can build Crux from source using: cargo install --path ." >&2
        exit 1
        ;;
esac

TARGET="${ARCH_TAG}-${OS_TAG}"
ARCHIVE_NAME="crux-${VERSION}-${TARGET}.tar.gz"
RELEASE_BASE_URL="${CRUX_RELEASE_BASE_URL:-https://github.com/${CRUX_REPO}/releases/download/${VERSION}}"

# Only HTTPS (or an explicit local file:// mirror) may serve release assets.
# Plain HTTP would let a network attacker replace both the archive and its
# SHA256SUMS, defeating verification.
case "$RELEASE_BASE_URL" in
    https://*|file://*) ;;
    *)
        echo "Error: release base URL must use https:// (or file:// for a local mirror): '$RELEASE_BASE_URL'" >&2
        exit 1
        ;;
esac
RELEASE_URL="${RELEASE_BASE_URL}/${ARCHIVE_NAME}"
CHECKSUMS_URL="${RELEASE_BASE_URL}/SHA256SUMS"

# 2. Check for required download tools
if command -v curl >/dev/null 2>&1; then
    DOWNLOAD_CMD="curl -fsSL"
elif command -v wget >/dev/null 2>&1; then
    DOWNLOAD_CMD="wget -qO-"
else
    echo "Error: Neither 'curl' nor 'wget' was found in PATH. Please install one of them." >&2
    exit 1
fi

# Check for sha256sum or shasum
if command -v sha256sum >/dev/null 2>&1; then
    SHA_CMD="sha256sum"
elif command -v shasum >/dev/null 2>&1; then
    SHA_CMD="shasum -a 256"
else
    echo "Error: Neither 'sha256sum' nor 'shasum' was found in PATH." >&2
    exit 1
fi

# 3. Create isolated temporary directory with auto-cleanup
TMP_DIR="$(mktemp -d -t crux-install-XXXXXX)"
TMP_DEST=""
cleanup() {
    rm -rf "$TMP_DIR"
    if [[ -n "$TMP_DEST" ]]; then
        rm -f "$TMP_DEST"
    fi
}
trap cleanup EXIT INT TERM

echo "--> Target:      $TARGET"
echo "--> Install dir: $INSTALL_DIR"

# 4. Download release archive and SHA256SUMS
ARCHIVE_PATH="$TMP_DIR/$ARCHIVE_NAME"
CHECKSUMS_PATH="$TMP_DIR/SHA256SUMS"

echo "--> Downloading release archive..."
if command -v curl >/dev/null 2>&1; then
    # Restrict protocols, including redirects, so a redirect can never
    # downgrade the download to plain HTTP.
    CURL_ARGS=(--proto '=https,file' --proto-redir '=https' -fsSL)
    curl "${CURL_ARGS[@]}" "$RELEASE_URL" -o "$ARCHIVE_PATH" || {
        echo "Error: Failed to download release archive from:" >&2
        echo "  $RELEASE_URL" >&2
        exit 1
    }
    curl "${CURL_ARGS[@]}" "$CHECKSUMS_URL" -o "$CHECKSUMS_PATH" || {
        echo "Error: Failed to download checksum file from:" >&2
        echo "  $CHECKSUMS_URL" >&2
        exit 1
    }
else
    if [[ "$RELEASE_BASE_URL" != https://* ]]; then
        echo "Error: wget can only download https:// release URLs; install curl to use '$RELEASE_BASE_URL'" >&2
        exit 1
    fi
    wget -q --https-only "$RELEASE_URL" -O "$ARCHIVE_PATH" || {
        echo "Error: Failed to download release archive from:" >&2
        echo "  $RELEASE_URL" >&2
        exit 1
    }
    wget -q --https-only "$CHECKSUMS_URL" -O "$CHECKSUMS_PATH" || {
        echo "Error: Failed to download checksum file from:" >&2
        echo "  $CHECKSUMS_URL" >&2
        exit 1
    }
fi

# 5. Cryptographic SHA-256 verification
echo "--> Verifying SHA-256 checksum..."
# Strict parsing: every non-empty line must be `<64 hex digits><space><space|*><name>`
# (the GNU coreutils format), and exactly one line must name this archive.
# Names are compared literally, never as patterns.
EXPECTED_SHA=""
MATCH_COUNT=0
while IFS= read -r line || [[ -n "$line" ]]; do
    line="${line%$'\r'}"
    [[ -z "$line" ]] && continue
    if [[ ! "$line" =~ ^([0-9A-Fa-f]{64})\ [\ *](.+)$ ]]; then
        echo "Error: malformed line in SHA256SUMS; refusing to continue:" >&2
        echo "  $line" >&2
        exit 1
    fi
    if [[ "${BASH_REMATCH[2]}" == "$ARCHIVE_NAME" ]]; then
        MATCH_COUNT=$((MATCH_COUNT + 1))
        EXPECTED_SHA="$(printf '%s' "${BASH_REMATCH[1]}" | tr 'A-F' 'a-f')"
    fi
done < "$CHECKSUMS_PATH"

if [[ "$MATCH_COUNT" -eq 0 ]]; then
    echo "Error: SHA256SUMS contains no checksum for '$ARCHIVE_NAME'. Aborting." >&2
    exit 1
fi
if [[ "$MATCH_COUNT" -gt 1 ]]; then
    echo "Error: SHA256SUMS contains $MATCH_COUNT entries for '$ARCHIVE_NAME'; refusing an ambiguous checksum. Aborting." >&2
    exit 1
fi

ACTUAL_SHA="$($SHA_CMD "$ARCHIVE_PATH" | awk '{print $1}' | tr 'A-F' 'a-f')"
if [[ ! "$ACTUAL_SHA" =~ ^[0-9a-f]{64}$ ]]; then
    echo "Error: could not compute the SHA-256 of the downloaded archive. Aborting." >&2
    exit 1
fi

if [[ "$EXPECTED_SHA" != "$ACTUAL_SHA" ]]; then
    echo "Error: Cryptographic checksum mismatch!" >&2
    echo "  Expected: $EXPECTED_SHA" >&2
    echo "  Actual:   $ACTUAL_SHA" >&2
    echo "The downloaded archive may be corrupted or tampered with. Aborting." >&2
    exit 1
fi

echo "--> Checksum verified: $ACTUAL_SHA"

# 6. Extract archive
echo "--> Extracting release archive..."
tar -xzf "$ARCHIVE_PATH" -C "$TMP_DIR"

EXTRACTED_BIN="$TMP_DIR/crux-${VERSION}-${TARGET}/crux"
if [[ ! -f "$EXTRACTED_BIN" ]]; then
    # Fallback to direct extraction if root folder was flat
    EXTRACTED_BIN="$TMP_DIR/crux"
fi

if [[ ! -f "$EXTRACTED_BIN" ]]; then
    echo "Error: Release archive did not contain expected 'crux' binary." >&2
    exit 1
fi

# 7. Install binary safely
mkdir -p "$INSTALL_DIR"
DEST_BIN="$INSTALL_DIR/crux"

# Write via an exclusively created temporary file in the destination directory
# for atomic replacement; the trap removes it if anything fails.
TMP_DEST="$(mktemp "${INSTALL_DIR}/.crux.tmp.XXXXXX")"
cp "$EXTRACTED_BIN" "$TMP_DEST"
chmod 755 "$TMP_DEST"
mv -f "$TMP_DEST" "$DEST_BIN"
TMP_DEST=""

# 8. Verify installed executable
echo "--> Verifying installed binary..."
INSTALLED_VER="$("$DEST_BIN" --version 2>&1)" || {
    echo "Error: Installed binary failed to execute at '$DEST_BIN'." >&2
    exit 1
}

echo ""
echo "=== Successfully installed Crux ==="
echo "Executable: $DEST_BIN"
echo "Version:    $INSTALLED_VER"
echo ""

# Check PATH
case ":$PATH:" in
    *":$INSTALL_DIR:"*)
        echo "Crux is in your PATH. Run 'crux --help' to get started."
        ;;
    *)
        echo "Notice: '$INSTALL_DIR' is not in your current PATH."
        echo "Add it to your shell configuration:"
        echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
        ;;
esac
