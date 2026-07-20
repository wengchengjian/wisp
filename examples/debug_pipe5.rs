use std::process::{Command, Stdio};
use std::io::{Read, Write};

fn main() {
    let chrome = r"C:\Program Files\Google\Chrome\Application\chrome.exe";
    let user_data = std::env::temp_dir().join(format!("pr-dbg5-{}", std::process::id()));
    
    // Try HEADED mode (no headless flag) with simple Stdio::piped()
    let mut child = Command::new(chrome)
        .arg("--remote-debugging-pipe")
        .arg("--no-first-run")
        .arg("--no-sandbox")
        .arg("--disable-background-networking")
        .arg(format!("--user-data-dir={}", user_data.display()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed");
    
    println!("PID: {:?}", child.id());
    std::thread::sleep(std::time::Duration::from_secs(3));
    
    match child.try_wait() {
        Ok(Some(s)) => {
            println!("Exited: {s}");
            // Read stderr for error
            if let Some(mut stderr) = child.stderr.take() {
                let mut buf = String::new();
                stderr.read_to_string(&mut buf).ok();
                if !buf.is_empty() { println!("STDERR: {buf}"); }
            }
        }
        Ok(None) => {
            println!("RUNNING!");
            if let Some(mut stdin) = child.stdin.take() {
                let cmd = r#"{"id":1,"method":"Browser.getVersion","params":{}}"#;
                let mut msg = cmd.as_bytes().to_vec();
                msg.push(0);
                stdin.write_all(&msg).ok();
                stdin.flush().ok();
                println!("Sent!");
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
            if let Some(mut stdout) = child.stdout.take() {
                let mut buf = vec![0u8; 8192];
                match stdout.read(&mut buf) {
                    Ok(n) if n > 0 => println!("GOT: {}", String::from_utf8_lossy(&buf[..n]).trim_end_matches('\0')),
                    _ => println!("No response"),
                }
            }
        }
        Err(e) => println!("Err: {e}"),
    }
    child.kill().ok();
    let _ = std::fs::remove_dir_all(&user_data);
}
