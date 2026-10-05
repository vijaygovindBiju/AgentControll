'use strict';

const fs = require('fs');
const path = require('path');
const os = require('os');
const https = require('https');
const http = require('http');
const crypto = require('crypto');
const { execFileSync } = require('child_process');
const { getTargetTriple, getArchiveName } = require('./platform');

const VERSION = require('../package.json').version;
const GITHUB_REPO = 'vijaygovindBiju/AgentControll';

/**
 * Returns the directory where AgentControll binaries are cached.
 */
function getCacheDir(version = VERSION) {
  const baseCache = process.env.XDG_CACHE_HOME || path.join(os.homedir(), '.cache');
  return path.join(baseCache, 'agentcontrol', 'bin', `v${version}`);
}

/**
 * HTTP GET request helper that follows redirects.
 */
function downloadFile(url, destPath, maxRedirects = 5) {
  return new Promise((resolve, reject) => {
    if (maxRedirects < 0) {
      return reject(new Error(`Too many redirects when downloading ${url}`));
    }

    const client = url.startsWith('https:') ? https : http;
    const req = client.get(url, (res) => {
      if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
        const redirectUrl = new URL(res.headers.location, url).toString();
        res.resume(); // Drain stream
        return resolve(downloadFile(redirectUrl, destPath, maxRedirects - 1));
      }

      if (res.statusCode !== 200) {
        res.resume();
        return reject(new Error(`Download failed with HTTP status ${res.statusCode} from ${url}`));
      }

      const fileStream = fs.createWriteStream(destPath);
      res.pipe(fileStream);

      fileStream.on('finish', () => {
        fileStream.close(resolve);
      });

      fileStream.on('error', (err) => {
        fs.unlink(destPath, () => {});
        reject(err);
      });
    });

    req.on('error', (err) => {
      fs.unlink(destPath, () => {});
      reject(err);
    });
  });
}

/**
 * Calculates SHA256 checksum of a file.
 */
function getFileSha256(filePath) {
  return new Promise((resolve, reject) => {
    const hash = crypto.createHash('sha256');
    const stream = fs.createReadStream(filePath);
    stream.on('data', (data) => hash.update(data));
    stream.on('end', () => resolve(hash.digest('hex')));
    stream.on('error', reject);
  });
}

/**
 * Ensures the binary exists and is executable.
 * Downloads from GitHub releases if not already cached.
 *
 * @param {string} binaryName Name of the binary (e.g. 'agentcontroll', 'agent-control', 'agentcontrold', 'ac')
 * @returns {Promise<string>} Absolute path to the executable binary
 */
async function ensureBinary(binaryName = 'agentcontroll') {
  // 1. Check AGENTCONTROL_BIN_DIR override (useful for dev or custom installations)
  if (process.env.AGENTCONTROL_BIN_DIR) {
    const overridePath = path.join(process.env.AGENTCONTROL_BIN_DIR, binaryName);
    if (fs.existsSync(overridePath)) {
      return overridePath;
    }
  }

  // 2. Check local cache directory
  const cacheDir = getCacheDir(VERSION);
  const binaryPath = path.join(cacheDir, binaryName);

  if (fs.existsSync(binaryPath)) {
    try {
      fs.accessSync(binaryPath, fs.constants.X_OK);
      return binaryPath;
    } catch {
      // If not executable, make it executable
      fs.chmodSync(binaryPath, 0o755);
      return binaryPath;
    }
  }

  // 3. Check local workspace build directory if developing or testing within repo
  const localCandidates = [
    path.resolve(__dirname, '../../target/release', binaryName),
    path.resolve(__dirname, '../../target/debug', binaryName),
    path.resolve(__dirname, '../../../target/release', binaryName),
    path.resolve(__dirname, '../../../target/debug', binaryName),
  ];
  for (const candidate of localCandidates) {
    if (fs.existsSync(candidate)) {
      try {
        fs.accessSync(candidate, fs.constants.X_OK);
        return candidate;
      } catch {
        try {
          fs.chmodSync(candidate, 0o755);
          return candidate;
        } catch {}
      }
    }
  }

  // 4. Need to download archive
  const targetTriple = getTargetTriple();
  const archiveName = getArchiveName(VERSION, targetTriple);
  const baseUrl = process.env.AGENTCONTROL_DOWNLOAD_URL_BASE ||
    `https://github.com/${GITHUB_REPO}/releases/download/v${VERSION}`;
  const archiveUrl = `${baseUrl}/${archiveName}`;

  fs.mkdirSync(cacheDir, { recursive: true });
  const tempArchive = path.join(cacheDir, `${archiveName}.tmp`);

  process.stderr.write(`[agentcontroll] Downloading AgentControll v${VERSION} for ${targetTriple}...\n`);

  try {
    await downloadFile(archiveUrl, tempArchive);

    // Extract using system tar
    process.stderr.write(`[agentcontroll] Extracting binary archive...\n`);
    execFileSync('tar', ['-xzf', tempArchive, '-C', cacheDir]);

    // Ensure permissions
    const binaries = ['agentcontroll', 'agent-control', 'agentcontrold', 'ac'];
    for (const bin of binaries) {
      const p = path.join(cacheDir, bin);
      if (fs.existsSync(p)) {
        fs.chmodSync(p, 0o755);
      }
    }

    // Clean up temporary archive
    try {
      fs.unlinkSync(tempArchive);
    } catch {}

    if (!fs.existsSync(binaryPath)) {
      throw new Error(`Expected binary '${binaryName}' was not found in downloaded release archive.`);
    }

    process.stderr.write(`[agentcontroll] Ready.\n`);
    return binaryPath;
  } catch (err) {
    try {
      if (fs.existsSync(tempArchive)) fs.unlinkSync(tempArchive);
    } catch {}

    // 5. Fallback: check if any other cached version exists before giving up
    const baseBinDir = path.dirname(cacheDir);
    if (fs.existsSync(baseBinDir)) {
      try {
        const versions = fs.readdirSync(baseBinDir)
          .filter((v) => v.startsWith('v') && fs.existsSync(path.join(baseBinDir, v, binaryName)))
          .sort()
          .reverse();
        if (versions.length > 0) {
          const fallbackVersion = versions[0];
          const fallbackPath = path.join(baseBinDir, fallbackVersion, binaryName);
          process.stderr.write(`[agentcontroll] Warning: Could not download v${VERSION} (${err.message}). Using cached ${fallbackVersion} binary.\n`);
          return fallbackPath;
        }
      } catch {}
    }

    throw new Error(
      `Failed to download prebuilt AgentControll binary from ${archiveUrl}:\n${err.message}\n` +
      `You can build from source using 'cargo build --release' or set AGENTCONTROL_BIN_DIR.`
    );
  }
}

module.exports = {
  getCacheDir,
  downloadFile,
  getFileSha256,
  ensureBinary,
};
