use std::process::{Command, Stdio};
use std::io::{Read, Write};

fn main() {
    // Test: launch Chrome with --remote-debugging-pipe and see what happens
    let chrome = r"C:\Program Files\Google\Chrome\Application\chrome.exe";
    let user_data = std::env::temp_dir().join(format!("pr-dbg-{}", std::process::id()));
    
    let mut child = Command::new(chrome)
        .arg("--remote-debugging-pipe")
        .arg("--headless=new")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-background-networking")
        .arg("--disable-sync")
        .arg("--no-sandbox")
        .arg(format!("--user-data-dir={}", user_data.display()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn chrome");
    
    println!("Chrome spawned, pid: {:?}", child.id());
    
    // Wait a bit
    std::thread::sleep(std::time::Duration::from_secs(2));
    
    // Check if still alive
    match child.try_wait() {
        Ok(Some(status)) => println!("Chrome exited with: {status}"),
        Ok(None) => println!("Chrome still running"),
        Err(e) => println!("Error checking: {e}"),
    }
    
    // Try to read stderr
    if let Some(mut stderr) = child.stderr.take() {
        let mut buf = vec![0u8; 4096];
        // Non-blocking read attempt
        match stderr.read(&mut buf) {
            Ok(0) => println!("stderr: empty"),
            Ok(n) => println!("stderr: {}", String::from_utf8_lossy(&buf[..n])),
            Err(e) => println!("stderr read error: {e}"),
        }
    }
    
    // Try to write a CDP command to stdin
    if let Some(mut stdin) = child.stdin.take() {
        let cmd = r#"{"id":1,"method":"Browser.getVersion","params":{}}"#;
        let mut msg = cmd.as_bytes().to_vec();
        msg.push(0);
        match stdin.write_all(&msg) {
            Ok(()) => {
                println!("Wrote CDP command to stdin");
                stdin.flush().ok();
            }
            Err(e) => println!("stdin write error: {e}"),
        }
    }
    
    // Wait for response on stdout
    std::thread::sleep(std::time::Duration::from_secs(1));
    if let Some(mut stdout) = child.stdout.take() {
        let mut buf = vec![0u8; 8192];
        match stdout.read(&mut buf) {
            Ok(0) => println!("stdout: empty (no response)"),
            Ok(n) => {
                let response = String::from_utf8_lossy(&buf[..n]);
                println!("stdout response: {}", response.trim_end_matches('\0'));
            }
            Err(e) => println!("stdout read error: {e}"),
        }
    }
    
    child.kill().ok();
    let _ = std::fs::remove_dir_all(&user_data);
}
