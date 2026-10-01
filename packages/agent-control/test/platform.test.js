'use strict';

const assert = require('assert');
const { getTargetTriple, getArchiveName, SUPPORTED_TARGETS } = require('../lib/platform');
const { getCacheDir } = require('../lib/downloader');

console.log('Running npm package unit tests...');

// Test 1: Supported targets mapping
assert.strictEqual(SUPPORTED_TARGETS['linux-x64'], 'x86_64-unknown-linux-gnu');
assert.strictEqual(SUPPORTED_TARGETS['darwin-x64'], 'x86_64-apple-darwin');
assert.strictEqual(SUPPORTED_TARGETS['darwin-arm64'], 'aarch64-apple-darwin');
console.log('✓ Target triple mappings are valid');

// Test 2: Archive name construction
const archiveName = getArchiveName('1.0.0', 'x86_64-unknown-linux-gnu');
assert.strictEqual(archiveName, 'agentcontroll-v1.0.0-x86_64-unknown-linux-gnu.tar.gz');
console.log('✓ Archive name formatting matches release contract');


// Test 3: Windows platform error handling
const oldPlatform = process.env.AGENTCONTROL_PLATFORM;
const oldArch = process.env.AGENTCONTROL_ARCH;

try {
  process.env.AGENTCONTROL_PLATFORM = 'win32';
  process.env.AGENTCONTROL_ARCH = 'x64';

  assert.throws(
    () => getTargetTriple(),
    /WSL2/
  );
  console.log('✓ win32 produces actionable WSL2 guidance');
} finally {
  if (oldPlatform !== undefined) {
    process.env.AGENTCONTROL_PLATFORM = oldPlatform;
  } else {
    delete process.env.AGENTCONTROL_PLATFORM;
  }
  if (oldArch !== undefined) {
    process.env.AGENTCONTROL_ARCH = oldArch;
  } else {
    delete process.env.AGENTCONTROL_ARCH;
  }
}

// Test 4: Unsupported platform error handling
try {
  process.env.AGENTCONTROL_PLATFORM = 'freebsd';
  process.env.AGENTCONTROL_ARCH = 'x64';

  assert.throws(
    () => getTargetTriple(),
    /Unsupported platform\/architecture/
  );
  console.log('✓ Unsupported platforms produce actionable error message');
} finally {
  delete process.env.AGENTCONTROL_PLATFORM;
  delete process.env.AGENTCONTROL_ARCH;
}

// Test 5: Cache dir structure
const cacheDir = getCacheDir('1.0.0');
assert.ok(cacheDir.includes('agentcontrol'));
assert.ok(cacheDir.endsWith('v1.0.0'));
console.log('✓ Cache directory conforms to XDG structure');

console.log('All npm package tests passed successfully!');
