#!/usr/bin/env sh
# Agent Control installer script
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/agentcontrol/agentcontrol/main/install.sh | sh
#
# Environment variables:
#   AGENTCONTROL_VERSION   Target version to install (defaults to "1.0.0")
#   BIN_DIR                Installation directory (defaults to ~/.local/bin or /usr/local/bin)
#   NO_MODIFY_PATH         Set to 1 to skip PATH guidance

set -e

REPO="agentcontrol/agentcontrol"
VERSION="${AGENTCONTROL_VERSION:-1.0.0}"

# Color output helpers if terminal supports colors
if [ -t 1 ]; then
  BOLD="\033[1m"
  GREEN="\033[32m"
  BLUE="\033[34m"
  YELLOW="\033[33m"
  RED="\033[31m"
  RESET="\033[0m"
else
  BOLD=""
  GREEN=""
  BLUE=""
  YELLOW=""
  RED=""
  RESET=""
fi

log_info() {
  printf "${BLUE}[agent-control]${RESET} %s\n" "$1"
}

log_success() {
  printf "${GREEN}[agent-control]${RESET} %s\n" "$1"
}

log_warn() {
  printf "${YELLOW}[agent-control] WARNING:${RESET} %s\n" "$1"
}

log_error() {
  printf "${RED}[agent-control] ERROR:${RESET} %s\n" "$1" >&2
}

# 1. Detect Operating System
OS="$(uname -s)"
case "$OS" in
  Linux)
    PLATFORM="unknown-linux-gnu"
    ;;
  Darwin)
    PLATFORM="apple-darwin"
    ;;
  CYGWIN*|MINGW*|MSYS*)
    log_error "Windows is not supported directly due to POSIX socket and PTY requirements."
    printf "Please install Agent Control inside WSL2 (Windows Subsystem for Linux):\n"
    printf "  1. Open PowerShell and run: wsl --install\n"
    printf "  2. Inside WSL2 terminal, run: curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | sh\n"
    exit 1
    ;;
  *)
    log_error "Unsupported operating system: $OS"
    printf "You can compile Agent Control from source using:\n"
    printf "  git clone https://github.com/${REPO}.git && cd agentcontrol && cargo build --release\n"
    exit 1
    ;;
esac

# 2. Detect CPU Architecture
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64|amd64)
    ARCH_TARGET="x86_64"
    ;;
  aarch64|arm64)
    ARCH_TARGET="aarch64"
    ;;
  *)
    log_error "Unsupported CPU architecture: $ARCH"
    exit 1
    ;;
esac

TARGET="${ARCH_TARGET}-${PLATFORM}"
ARCHIVE_NAME="agent-control-v${VERSION}-${TARGET}.tar.gz"
DOWNLOAD_URL="https://github.com/${REPO}/releases/download/v${VERSION}/${ARCHIVE_NAME}"

# 3. Determine Installation Directory
if [ -n "$BIN_DIR" ]; then
  INSTALL_DIR="$BIN_DIR"
elif [ "$(id -u)" -eq 0 ]; then
  INSTALL_DIR="/usr/local/bin"
else
  INSTALL_DIR="${HOME}/.local/bin"
fi

mkdir -p "$INSTALL_DIR"

# 4. Prepare temporary directory
TMP_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t 'agentcontrol')"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

log_info "Downloading Agent Control v${VERSION} for ${TARGET}..."

if command -v curl >/dev/null 2>&1; then
  curl -fL --progress-bar "$DOWNLOAD_URL" -o "${TMP_DIR}/${ARCHIVE_NAME}"
elif command -v wget >/dev/null 2>&1; then
  wget -q --show-progress "$DOWNLOAD_URL" -O "${TMP_DIR}/${ARCHIVE_NAME}"
else
  log_error "Neither curl nor wget is available. Please install curl or wget."
  exit 1
fi

log_info "Extracting archive to ${INSTALL_DIR}..."
tar -xzf "${TMP_DIR}/${ARCHIVE_NAME}" -C "$INSTALL_DIR"

# Ensure executables have appropriate permissions
chmod 0755 "${INSTALL_DIR}/agent-control" 2>/dev/null || true
chmod 0755 "${INSTALL_DIR}/agentcontrold" 2>/dev/null || true
chmod 0755 "${INSTALL_DIR}/agy" 2>/dev/null || true
chmod 0755 "${INSTALL_DIR}/ac" 2>/dev/null || true

log_success "Agent Control v${VERSION} installed successfully to ${INSTALL_DIR}!"

# 5. Verify PATH
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    if [ "$NO_MODIFY_PATH" != "1" ]; then
      log_warn "${INSTALL_DIR} is not in your system PATH."
      printf "Add the following line to your shell configuration file (~/.bashrc, ~/.zshrc, etc.):\n\n"
      printf "  ${BOLD}export PATH=\"%s:\$PATH\"${RESET}\n\n" "$INSTALL_DIR"
    fi
    ;;
esac

printf "\nRun '${BOLD}agent-control${RESET}' or '${BOLD}agy${RESET}' to get started!\n"
