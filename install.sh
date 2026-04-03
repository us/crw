#!/bin/sh
# CRW installer — downloads the latest release binary for your platform.
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/us/crw/main/install.sh | sh
#   wget -qO- https://raw.githubusercontent.com/us/crw/main/install.sh | sh
#
# Options (environment variables):
#   CRW_VERSION=v0.3.0    Install a specific version instead of latest
#   CRW_INSTALL_DIR=~/.local/bin   Custom install directory
#   GITHUB_TOKEN=ghp_...  Avoid GitHub API rate limits

set -eu

main() {

REPO="us/crw"
INSTALL_DIR="${CRW_INSTALL_DIR:-/usr/local/bin}"
BINARY="${CRW_BINARY:-crw-mcp}"

# --- helpers ----------------------------------------------------------------

BOLD="$(tput bold 2>/dev/null || printf '')"
BLUE="$(tput setaf 4 2>/dev/null || printf '')"
GREEN="$(tput setaf 2 2>/dev/null || printf '')"
RED="$(tput setaf 1 2>/dev/null || printf '')"
RESET="$(tput sgr0 2>/dev/null || printf '')"

info()      { printf '%s==>%s %s\n' "${BLUE}${BOLD}" "${RESET}" "$*"; }
success()   { printf '%s==>%s %s\n' "${GREEN}${BOLD}" "${RESET}" "$*"; }
err()       { printf '%serror:%s %s\n' "${RED}${BOLD}" "${RESET}" "$*" >&2; exit 1; }

need() {
  command -v "$1" >/dev/null 2>&1 || err "'$1' is required but not found"
}

# --- detect downloader ------------------------------------------------------

download() {
  if command -v curl >/dev/null 2>&1; then
    curl --fail --location --silent --show-error \
         --proto '=https' --tlsv1.2 \
         --output "$2" "$1"
  elif command -v wget >/dev/null 2>&1; then
    wget --https-only --quiet --output-document="$2" "$1"
  else
    err "curl or wget is required"
  fi
}

# --- detect platform --------------------------------------------------------

detect_platform() {
  OS="$(uname -s)"
  ARCH="$(uname -m)"

  case "$OS" in
    Darwin)  PLATFORM="darwin" ;;
    Linux)   PLATFORM="linux"  ;;
    MINGW*|MSYS*|CYGWIN*) PLATFORM="win32" ;;
    *)       err "Unsupported OS: $OS. Try: cargo install crw-mcp" ;;
  esac

  # Rosetta 2 detection — uname returns x86_64 under Rosetta on Apple Silicon
  if [ "$PLATFORM" = "darwin" ] && [ "$ARCH" = "x86_64" ]; then
    if sysctl -n sysctl.proc_translated 2>/dev/null | grep -q '^1$'; then
      info "Rosetta 2 detected — installing native arm64 binary"
      ARCH="arm64"
    fi
  fi

  case "$ARCH" in
    x86_64|amd64)  ARCH_LABEL="x64"   ;;
    aarch64|arm64) ARCH_LABEL="arm64"  ;;
    *)             err "Unsupported architecture: $ARCH. Try: cargo install crw-mcp" ;;
  esac

  # musl libc detection — pre-built binaries require glibc
  if [ "$PLATFORM" = "linux" ]; then
    if command -v ldd >/dev/null 2>&1 && ldd --version 2>&1 | grep -qi musl; then
      err "musl libc detected (Alpine Linux?). Pre-built binaries require glibc. Try: cargo install crw-mcp"
    fi
  fi

  if [ "$PLATFORM" = "win32" ]; then
    ASSET="${BINARY}-${PLATFORM}-${ARCH_LABEL}.zip"
  else
    ASSET="${BINARY}-${PLATFORM}-${ARCH_LABEL}.tar.gz"
  fi
}

# --- fetch latest version ---------------------------------------------------

get_version() {
  # Allow pinning to a specific version
  if [ -n "${CRW_VERSION:-}" ]; then
    VERSION="$CRW_VERSION"
    return
  fi

  AUTH_HEADER=""
  if [ -n "${GITHUB_TOKEN:-}" ]; then
    AUTH_HEADER="Authorization: token ${GITHUB_TOKEN}"
  fi

  CRW_TMPFILE="$(mktemp)"
  if command -v curl >/dev/null 2>&1; then
    if [ -n "$AUTH_HEADER" ]; then
      curl -fsSL -H "$AUTH_HEADER" \
        "https://api.github.com/repos/${REPO}/releases/latest" > "$CRW_TMPFILE" 2>/dev/null || true
    else
      curl -fsSL \
        "https://api.github.com/repos/${REPO}/releases/latest" > "$CRW_TMPFILE" 2>/dev/null || true
    fi
  elif command -v wget >/dev/null 2>&1; then
    if [ -n "$AUTH_HEADER" ]; then
      wget --header="$AUTH_HEADER" --quiet \
        -O "$CRW_TMPFILE" "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null || true
    else
      wget --quiet \
        -O "$CRW_TMPFILE" "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null || true
    fi
  else
    rm -f "$CRW_TMPFILE"
    err "curl or wget is required"
  fi

  # Check for rate limiting
  if grep -q '"rate limit"' "$CRW_TMPFILE" 2>/dev/null; then
    rm -f "$CRW_TMPFILE"
    err "GitHub API rate limit exceeded. Set GITHUB_TOKEN or use: CRW_VERSION=v0.3.0"
  fi

  VERSION=$(grep '"tag_name"' "$CRW_TMPFILE" | head -1 | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')
  rm -f "$CRW_TMPFILE"
  [ -n "$VERSION" ] || err "Could not determine latest version. Use: CRW_VERSION=v0.3.0"
}

# --- download & install -----------------------------------------------------

install() {
  URL="https://github.com/${REPO}/releases/download/${VERSION}/${ASSET}"
  CRW_TMPDIR="$(mktemp -d)"
  trap 'rm -rf "$CRW_TMPDIR"' EXIT

  # Check for existing installation
  if command -v "$BINARY" >/dev/null 2>&1; then
    INSTALLED=$("$BINARY" --version 2>/dev/null | head -1 || echo "unknown")
    info "Upgrading from ${INSTALLED} to ${VERSION}..."
  else
    info "Downloading CRW ${VERSION} (${PLATFORM}/${ARCH_LABEL})..."
  fi

  download "$URL" "${CRW_TMPDIR}/${ASSET}"

  info "Extracting..."
  if [ "$PLATFORM" = "win32" ]; then
    need unzip
    unzip -o "${CRW_TMPDIR}/${ASSET}" -d "$CRW_TMPDIR" >/dev/null
  else
    tar xzf "${CRW_TMPDIR}/${ASSET}" -C "$CRW_TMPDIR"
  fi

  # Verify the binary was extracted
  [ -f "${CRW_TMPDIR}/${BINARY}" ] || err "Archive did not contain '${BINARY}'"

  # Create install directory if needed
  if [ ! -d "$INSTALL_DIR" ]; then
    if [ -w "$(dirname "$INSTALL_DIR")" ]; then
      mkdir -p "$INSTALL_DIR"
    else
      sudo mkdir -p "$INSTALL_DIR"
    fi
  fi

  info "Installing to ${INSTALL_DIR}/${BINARY}..."
  if [ -w "$INSTALL_DIR" ]; then
    mv "${CRW_TMPDIR}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
    chmod +x "${INSTALL_DIR}/${BINARY}"
  else
    sudo mv "${CRW_TMPDIR}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
    sudo chmod +x "${INSTALL_DIR}/${BINARY}"
  fi

  success "CRW ${VERSION} installed to ${INSTALL_DIR}/${BINARY}"
  echo ""
  echo "  Run:  ${BINARY} --help"
  echo ""

  # Check if install dir is in PATH
  case ":$PATH:" in
    *":${INSTALL_DIR}:"*) ;;
    *) echo "  Note: ${INSTALL_DIR} is not in your PATH. Add it with:"
       echo "    export PATH=\"${INSTALL_DIR}:\$PATH\""
       echo "" ;;
  esac
}

# --- run --------------------------------------------------------------------

detect_platform
get_version
install

}

# main() wrapper ensures the entire script is downloaded before execution
main
