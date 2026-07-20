// patchright-helper.js
// Spawns Chrome with --remote-debugging-pipe and forwards CDP messages
// between the parent process (Rust) via stdin/stdout and Chrome via fd 3/4.
//
// Protocol: null-byte delimited JSON on all channels.
// Parent (Rust) <-> this helper <-> Chrome

const { spawn } = require('child_process');

const args = process.argv.slice(2);
const chromePath = args[0];
const chromeArgs = args.slice(1);

if (!chromePath) {
  process.stderr.write('Usage: node patchright-helper.js <chrome_path> [args...]\n');
  process.exit(1);
}

// Spawn Chrome with 5 stdio entries (fd 3 and fd 4 for pipe CDP)
// windowsHide: false allows Chrome to create its window in headed mode
const chrome = spawn(chromePath, chromeArgs, {
  stdio: ['pipe', 'pipe', 'pipe', 'pipe', 'pipe'],
  windowsHide: false
});

// Forward CDP messages from parent (stdin) to Chrome (fd 3)
// Always forward regardless of Chrome's exit state - the pipe might still be open
process.stdin.on('data', (data) => {
  try {
    if (chrome.stdio[3] && chrome.stdio[3].writable) {
      chrome.stdio[3].write(data);
    }
  } catch(e) {}
});

process.stdin.on('end', () => {
  // Parent closed stdin - kill Chrome and exit
  try { chrome.kill(); } catch(e) {}
  process.exit(0);
});

// Keep stdin flowing
process.stdin.resume();

// Forward CDP messages from Chrome (fd 4) to parent (stdout)
if (chrome.stdio[4]) {
  chrome.stdio[4].on('data', (data) => {
    try { process.stdout.write(data); } catch(e) {}
  });
}

// Also listen on fd 3 for any data Chrome might send there
if (chrome.stdio[3]) {
  chrome.stdio[3].on('data', (data) => {
    try { process.stdout.write(data); } catch(e) {}
  });
}

// When Chrome exits, do NOT exit the helper.
// In headed mode, Chrome's launcher process may exit while the browser
// process keeps the pipe handles open. We stay alive until stdin closes.
chrome.on('exit', (code) => {
  // Log but don't exit
  process.stderr.write('[helper] chrome exited code=' + code + ', staying alive\n');
});

// Handle termination
process.on('SIGTERM', () => {
  try { chrome.kill(); } catch(e) {}
  process.exit(0);
});
