#!/usr/bin/env bash
# Package Agent Control release binaries into tarballs with SHA256 checksums
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

VERSION="${1:-$(grep -m 1 '^version = ' "${REPO_ROOT}/Cargo.toml" | awk -F '"' '{print $2}')}"
TARGET="${2:-$(rustc -vV | sed -n 's|host: ||p')}"
DIST_DIR="${DIST_DIR:-${REPO_ROOT}/dist}"


echo "=========================================================="
echo "Packaging Agent Control v${VERSION} for target ${TARGET}"
echo "=========================================================="

mkdir -p "${DIST_DIR}"

CARGO_TARGET_FLAG=""
TARGET_DIR="${REPO_ROOT}/target"
if [ -n "${CARGO_TARGET_DIR:-}" ]; then
  TARGET_DIR="${CARGO_TARGET_DIR}"
fi

BIN_DIR="${TARGET_DIR}/release"
if [ "${TARGET}" != "$(rustc -vV | sed -n 's|host: ||p')" ]; then
  CARGO_TARGET_FLAG="--target ${TARGET}"
  BIN_DIR="${TARGET_DIR}/${TARGET}/release"
fi

echo "Building release binaries..."
# Build without incremental compilation to save disk space
CARGO_INCREMENTAL=0 cargo build --release --workspace ${CARGO_TARGET_FLAG}

BINARIES=("agent-control" "agentcontrold" "agy" "ac")
for bin in "${BINARIES[@]}"; do
  if [ ! -f "${BIN_DIR}/${bin}" ]; then
    echo "Error: Binary '${BIN_DIR}/${bin}' not found." >&2
    exit 1
  fi
done

ARCHIVE_NAME="agent-control-v${VERSION}-${TARGET}.tar.gz"
ARCHIVE_PATH="${DIST_DIR}/${ARCHIVE_NAME}"

echo "Creating tarball: ${ARCHIVE_PATH}"
tar -czf "${ARCHIVE_PATH}" -C "${BIN_DIR}" "${BINARIES[@]}"

echo "Generating SHA256 checksum..."
cd "${DIST_DIR}"
if command -v sha256sum >/dev/null 2>&1; then
  sha256sum "${ARCHIVE_NAME}" >> SHA256SUMS
elif command -v shasum >/dev/null 2>&1; then
  shasum -a 256 "${ARCHIVE_NAME}" >> SHA256SUMS
fi

echo "Release package created successfully:"
ls -lh "${ARCHIVE_PATH}"
echo "Checksum recorded in ${DIST_DIR}/SHA256SUMS"
