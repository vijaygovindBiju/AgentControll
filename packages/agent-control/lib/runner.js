'use strict';

const { spawn } = require('child_process');
const { ensureBinary } = require('./downloader');

/**
 * Executes a target AgentControll binary, forwarding arguments, stdio, and signals.
 *
 * @param {string} binaryName Name of the AgentControll binary to run.
 */
async function runBinary(binaryName) {
  try {
    const binaryPath = await ensureBinary(binaryName);

    const child = spawn(binaryPath, process.argv.slice(2), {
      stdio: 'inherit',
    });

    // Forward terminal resize signal (vital for TUI layout updates)
    const onResize = () => {
      if (child.pid && !child.killed) {
        try {
          child.kill('SIGWINCH');
        } catch {}
      }
    };
    process.on('SIGWINCH', onResize);

    // Forward termination signals
    const onSignal = (sig) => {
      if (child.pid && !child.killed) {
        try {
          child.kill(sig);
        } catch {}
      }
    };
    process.on('SIGINT', () => onSignal('SIGINT'));
    process.on('SIGTERM', () => onSignal('SIGTERM'));

    child.on('close', (code, signal) => {
      process.removeListener('SIGWINCH', onResize);
      if (signal) {
        process.kill(process.pid, signal);
      } else {
        process.exit(code !== null ? code : 0);
      }
    });

    child.on('error', (err) => {
      console.error(`[agentcontroll] Failed to execute ${binaryName}: ${err.message}`);
      process.exit(1);
    });
  } catch (err) {
    console.error(`[agentcontroll] Error: ${err.message}`);
    process.exit(1);
  }
}

module.exports = {
  runBinary,
};
