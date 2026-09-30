'use strict';

const os = require('os');

/**
 * Supported release targets and their mapping to GitHub Release archives.
 */
const SUPPORTED_TARGETS = {
  'linux-x64': 'x86_64-unknown-linux-gnu',
  'linux-arm64': 'aarch64-unknown-linux-gnu',
  'darwin-x64': 'x86_64-apple-darwin',
  'darwin-arm64': 'aarch64-apple-darwin',
};

/**
 * Returns the target triple for the current system, or throws a helpful error.
 */
function getTargetTriple() {
  const platform = process.env.AGENTCONTROL_PLATFORM || os.platform();
  const arch = process.env.AGENTCONTROL_ARCH || os.arch();

  if (platform === 'win32') {
    throw new Error(
      'Agent Control requires POSIX Unix domain sockets (mode 0600) and native terminal signals.\n' +
      'Windows users should run Agent Control inside WSL2 (Windows Subsystem for Linux):\n' +
      '  wsl --install\n' +
      'Inside WSL2, run: npx agent-control'
    );
  }

  const key = `${platform}-${arch}`;
  const target = SUPPORTED_TARGETS[key];

  if (!target) {
    throw new Error(
      `Unsupported platform/architecture: ${platform}-${arch}.\n` +
      `Agent Control currently provides prebuilt binaries for:\n` +
      Object.keys(SUPPORTED_TARGETS).map(k => `  - ${k}`).join('\n') +
      `\nTo build from source on your platform, run: cargo build --release`
    );
  }

  return target;
}

/**
 * Returns the release archive name for a given version and target triple.
 */
function getArchiveName(version, target) {
  return `agent-control-v${version}-${target}.tar.gz`;
}

module.exports = {
  SUPPORTED_TARGETS,
  getTargetTriple,
  getArchiveName,
};
