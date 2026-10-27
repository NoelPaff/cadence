#!/usr/bin/env sh
# Cadence installer for Linux and macOS.
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/NoelPaff/cadence/main/install.sh | sh
#   ./install.sh [--from-source]

set -eu

REPO="NoelPaff/cadence"
INSTALL_DIR="${CADENCE_INSTALL_DIR:-$HOME/.local/bin}"
FROM_SOURCE=0

for arg in "$@"; do
    case "$arg" in
        --from-source) FROM_SOURCE=1 ;;
        -h|--help)
            echo "Usage: install.sh [--from-source]"
            echo "Environment variables:"
            echo "  CADENCE_INSTALL_DIR  Target directory (default: ~/.local/bin)"
            exit 0
            ;;
    esac
done

mkdir -p "$INSTALL_DIR"

install_from_source() {
    if ! command -v cargo >/dev/null 2>&1; then
        echo "Error: 'cargo' (Rust toolchain) is not installed." >&2
        echo "Install Rust from https://rustup.rs/ and re-run this script." >&2
        exit 1
    fi
    if [ -f "Cargo.toml" ] && grep -q 'name = "cadence"' Cargo.toml 2>/dev/null; then
        echo "==> Building Cadence from local source..."
        cargo build --release --locked
        cp target/release/cadence "$INSTALL_DIR/cadence"
        chmod 755 "$INSTALL_DIR/cadence"
    else
        echo "==> Compiling Cadence from https://github.com/$REPO ..."
        TMP_ROOT="$(mktemp -d)"
        trap 'rm -rf "$TMP_ROOT"' EXIT
        cargo install --git "https://github.com/$REPO.git" --locked --root "$TMP_ROOT"
        cp "$TMP_ROOT/bin/cadence" "$INSTALL_DIR/cadence"
        chmod 755 "$INSTALL_DIR/cadence"
    fi
}

try_prebuilt_download() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Linux)  OS_TAG="unknown-linux-gnu" ;;
        Darwin) OS_TAG="apple-darwin" ;;
        *)      return 1 ;;
    esac

    case "$ARCH" in
        x86_64|amd64)  ARCH_TAG="x86_64" ;;
        aarch64|arm64) ARCH_TAG="aarch64" ;;
        *)             return 1 ;;
    esac

    ASSET="cadence-${ARCH_TAG}-${OS_TAG}.tar.gz"
    URL="https://github.com/$REPO/releases/latest/download/$ASSET"

    TMP_DIR="$(mktemp -d)"
    echo "==> Downloading prebuilt binary ($ASSET)..."
    if command -v curl >/dev/null 2>&1; then
        if ! curl -fsSL "$URL" -o "$TMP_DIR/$ASSET"; then
            rm -rf "$TMP_DIR"
            return 1
        fi
    elif command -v wget >/dev/null 2>&1; then
        if ! wget -qO "$TMP_DIR/$ASSET" "$URL"; then
            rm -rf "$TMP_DIR"
            return 1
        fi
    else
        rm -rf "$TMP_DIR"
        return 1
    fi

    tar -xzf "$TMP_DIR/$ASSET" -C "$TMP_DIR"
    cp "$TMP_DIR/cadence" "$INSTALL_DIR/cadence"
    chmod 755 "$INSTALL_DIR/cadence"
    rm -rf "$TMP_DIR"
    return 0
}

if [ "$FROM_SOURCE" -eq 1 ]; then
    install_from_source
else
    if ! try_prebuilt_download; then
        echo "==> Prebuilt archive unavailable for this platform; building from source..."
        install_from_source
    fi
fi

echo ""
echo "✓ Installed Cadence to: $INSTALL_DIR/cadence"
case ":$PATH:" in
    *":$INSTALL_DIR:"*)
        echo "Run 'cadence' to launch!"
        ;;
    *)
        echo "Add $INSTALL_DIR to your PATH to run 'cadence' from anywhere:"
        echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
        ;;
esac
