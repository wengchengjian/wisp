const { spawn } = require('child_process');
const chrome = 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe';
const userData = process.env.TEMP + '\\pr-node-' + process.pid;

const child = spawn(chrome, [
  '--remote-debugging-pipe',
  '--headless=new',
  '--no-first-run',
  '--no-sandbox',
  '--disable-background-networking',
  '--user-data-dir=' + userData
], {
  stdio: ['pipe', 'pipe', 'pipe', 'pipe', 'pipe']  // fd 3 and fd 4 for Chrome pipe
});

console.log('PID:', child.pid);
child.stderr.on('data', d => console.log('STDERR:', d.toString().trim()));

setTimeout(() => {
  if (child.exitCode !== null) {
    console.log('EXITED with code:', child.exitCode);
    process.exit(1);
  }
  console.log('RUNNING! Trying both fd directions...');
  
  // Listen on both fd3 and fd4
  child.stdio[3].on('data', d => console.log('FD3 DATA:', d.toString().replace(/\0/g, '').substring(0, 200)));
  child.stdio[4].on('data', d => console.log('FD4 DATA:', d.toString().replace(/\0/g, '').substring(0, 200)));
  
  const cmd = JSON.stringify({id:1, method:'Browser.getVersion', params:{}}) + '\0';
  
  // Try writing to fd4 first (Playwright style)
  console.log('Writing to fd4...');
  child.stdio[4].write(cmd);
  
  // Also try fd3 after a delay
  setTimeout(() => {
    console.log('Also writing to fd3...');
    child.stdio[3].write(cmd);
  }, 1000);
}, 3000);

setTimeout(() => { console.log('TIMEOUT'); child.kill(); process.exit(1); }, 10000);
