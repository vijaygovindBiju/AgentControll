#!/usr/bin/env sh
# AgentControll installer script
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/vijaygovindBiju/AgentControll/main/install.sh | sh
#
# Environment variables:
#   AGENTCONTROL_VERSION   Target version to install (defaults to "1.0.0")
#   BIN_DIR                Installation directory (defaults to ~/.local/bin or /usr/local/bin)
#   NO_MODIFY_PATH         Set to 1 to skip PATH guidance

set -e

REPO="vijaygovindBiju/AgentControll"
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
  printf "${BLUE}[agentcontroll]${RESET} %s\n" "$1"
}

log_success() {
  printf "${GREEN}[agentcontroll]${RESET} %s\n" "$1"
}

log_warn() {
  printf "${YELLOW}[agentcontroll] WARNING:${RESET} %s\n" "$1"
}

log_error() {
  printf "${RED}[agentcontroll] ERROR:${RESET} %s\n" "$1" >&2
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
    printf "Please install AgentControll inside WSL2 (Windows Subsystem for Linux):\n"
    printf "  1. Open PowerShell and run: wsl --install\n"
    printf "  2. Inside WSL2 terminal, run: curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | sh\n"
    exit 1
    ;;
  *)
    log_error "Unsupported operating system: $OS"
    printf "You can compile AgentControll from source using:\n"
    printf "  git clone https://github.com/${REPO}.git && cd AgentControll && cargo build --release\n"
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
    if [ "$PLATFORM" = "apple-darwin" ]; then
      ARCH_TARGET="aarch64"
    else
      log_error "Linux ARM64 (aarch64) prebuilt release binaries are not currently provided."
      printf "You can compile AgentControll from source using:\n"
      printf "  git clone https://github.com/${REPO}.git && cd AgentControll && cargo build --release\n"
      exit 1
    fi
    ;;
  *)
    log_error "Unsupported CPU architecture: $ARCH"
    exit 1
    ;;
esac

TARGET="${ARCH_TARGET}-${PLATFORM}"
ARCHIVE_NAME="agentcontroll-v${VERSION}-${TARGET}.tar.gz"
DOWNLOAD_URL="https://github.com/${REPO}/releases/download/v${VERSION}/${ARCHIVE_NAME}"
CHECKSUMS_URL="https://github.com/${REPO}/releases/download/v${VERSION}/SHA256SUMS"

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
TMP_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t 'agentcontroll')"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

log_info "Downloading AgentControll v${VERSION} for ${TARGET}..."

if command -v curl >/dev/null 2>&1; then
  curl -fL --progress-bar "$DOWNLOAD_URL" -o "${TMP_DIR}/${ARCHIVE_NAME}"
elif command -v wget >/dev/null 2>&1; then
  wget -q --show-progress "$DOWNLOAD_URL" -O "${TMP_DIR}/${ARCHIVE_NAME}"
else
  log_error "Neither curl nor wget is available. Please install curl or wget."
  exit 1
fi

# 5. Checksum Verification
log_info "Verifying SHA256 checksum..."
if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$CHECKSUMS_URL" -o "${TMP_DIR}/SHA256SUMS" 2>/dev/null || true
elif command -v wget >/dev/null 2>&1; then
  wget -q "$CHECKSUMS_URL" -O "${TMP_DIR}/SHA256SUMS" 2>/dev/null || true
fi

if [ -f "${TMP_DIR}/SHA256SUMS" ]; then
  (
    cd "$TMP_DIR"
    if command -v sha256sum >/dev/null 2>&1; then
      if grep -F " ${ARCHIVE_NAME}" SHA256SUMS >/dev/null 2>&1; then
        grep -F " ${ARCHIVE_NAME}" SHA256SUMS | sha256sum -c --status || {
          log_error "SHA256 checksum verification failed for ${ARCHIVE_NAME}!"
          exit 1
        }
      fi
    elif command -v shasum >/dev/null 2>&1; then
      if grep -F " ${ARCHIVE_NAME}" SHA256SUMS >/dev/null 2>&1; then
        grep -F " ${ARCHIVE_NAME}" SHA256SUMS | shasum -a 256 -c --status || {
          log_error "SHA256 checksum verification failed for ${ARCHIVE_NAME}!"
          exit 1
        }
      fi
    fi
  )
fi

log_info "Extracting archive to ${INSTALL_DIR}..."
tar -xzf "${TMP_DIR}/${ARCHIVE_NAME}" -C "$INSTALL_DIR"

# Ensure executables have appropriate permissions
chmod 0755 "${INSTALL_DIR}/agentcontroll" 2>/dev/null || true
chmod 0755 "${INSTALL_DIR}/agent-control" 2>/dev/null || true
chmod 0755 "${INSTALL_DIR}/agentcontrold" 2>/dev/null || true
chmod 0755 "${INSTALL_DIR}/agy" 2>/dev/null || true
chmod 0755 "${INSTALL_DIR}/ac" 2>/dev/null || true

log_success "AgentControll v${VERSION} installed successfully to ${INSTALL_DIR}!"

# 6. Verify PATH
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

printf "\nRun '${BOLD}agentcontroll${RESET}' or '${BOLD}agy${RESET}' to get started!\n"
