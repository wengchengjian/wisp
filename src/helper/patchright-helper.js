// patchright-helper.js
// Spawns Chrome with --remote-debugging-pipe and forwards CDP messages
// between the parent process (Rust) via stdin/stdout and Chrome via fd 3/4.
//
// Protocol: null-byte delimited JSON on all channels.
// Parent (Rust) <-> this helper <-> Chrome

const { spawn } = require('child_process');

// Parse args: node patchright-helper.js <chrome_path> [args...]
const args = process.argv.slice(2);
const chromePath = args[0];
const chromeArgs = args.slice(1);

if (!chromePath) {
  process.stderr.write('Usage: node patchright-helper.js <chrome_path> [args...]\n');
  process.exit(1);
}

// Spawn Chrome with 5 stdio entries (fd 3 and fd 4 for pipe CDP)
const chrome = spawn(chromePath, chromeArgs, {
  stdio: ['pipe', 'pipe', 'pipe', 'pipe', 'pipe']
});

let chromeAlive = true;

chrome.on('exit', (code) => {
  // In headed mode, Chrome's launcher process may exit after spawning
  // the real browser process. Don't exit immediately - wait to see if
  // the pipe is still active.
  process.stderr.write('[helper] chrome process exited with code ' + code + '\n');
  chromeAlive = false;
  // Give time for any remaining pipe data to arrive
  setTimeout(() => {
    process.exit(code || 0);
  }, 2000);
});

chrome.stderr.on('data', (d) => {
  // Forward Chrome stderr to our stderr (for debugging)
  process.stderr.write(d);
});

// Forward CDP messages from parent (stdin) to Chrome (fd 3)
// Chrome reads commands from fd 3
process.stdin.on('data', (data) => {
  process.stderr.write('[helper] stdin recv ' + data.length + ' bytes\n');
  if (chromeAlive && chrome.stdio[3] && chrome.stdio[3].writable) {
    chrome.stdio[3].write(data);
    process.stderr.write('[helper] forwarded to chrome fd3\n');
  } else {
    process.stderr.write('[helper] chrome fd3 not writable!\n');
  }
});

process.stdin.on('end', () => {
  // Parent closed stdin - wait a bit before killing Chrome
  // (might be a temporary state)
  process.stderr.write('[helper] stdin ended\n');
  setTimeout(() => {
    if (chromeAlive) chrome.kill();
    process.exit(0);
  }, 1000);
});

// Keep the process alive
process.stdin.resume();

// Forward CDP messages from Chrome (fd 4) to parent (stdout)
// Chrome writes responses to fd 4
if (chrome.stdio[4] && chrome.stdio[4].readable) {
  chrome.stdio[4].on('data', (data) => {
    process.stderr.write('[helper] chrome fd4 recv ' + data.length + ' bytes\n');
    try { process.stdout.write(data); } catch(e) {}
  });
}

// Also listen on fd 3 for any data Chrome might send there
if (chrome.stdio[3] && chrome.stdio[3].readable) {
  chrome.stdio[3].on('data', (data) => {
    process.stderr.write('[helper] chrome fd3 recv ' + data.length + ' bytes\n');
    try { process.stdout.write(data); } catch(e) {}
  });
}

// Keep the process alive
process.on('SIGTERM', () => {
  if (chromeAlive) chrome.kill();
  process.exit(0);
});
