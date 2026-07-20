use std::process::{Command, Stdio};
use std::io::{Read, Write};
use std::os::windows::io::AsRawHandle;

fn main() {
    let chrome = r"C:\Program Files\Google\Chrome\Application\chrome.exe";
    let user_data = std::env::temp_dir().join(format!("pr-dbg2-{}", std::process::id()));
    
    // Try: pass pipe handle values explicitly
    // First, create pipes and get their handle values
    let mut child = Command::new(chrome)
        .arg("--remote-debugging-pipe")
        .arg("--headless=new")
        .arg("--no-first-run")
        .arg("--no-sandbox")
        .arg("--disable-background-networking")
        .arg(format!("--user-data-dir={}", user_data.display()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())  // Let stderr go to console for debugging
        .spawn()
        .expect("failed to spawn");
    
    println!("PID: {:?}", child.id());
    std::thread::sleep(std::time::Duration::from_secs(2));
    
    match child.try_wait() {
        Ok(Some(s)) => println!("Exited: {s}"),
        Ok(None) => {
            println!("Running! Trying CDP...");
            if let Some(mut stdin) = child.stdin.take() {
                let cmd = r#"{"id":1,"method":"Browser.getVersion","params":{}}"#;
                let mut msg = cmd.as_bytes().to_vec();
                msg.push(0);
                match stdin.write_all(&msg) {
                    Ok(()) => { println!("Sent command"); stdin.flush().ok(); }
                    Err(e) => println!("Write error: {e}"),
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
            if let Some(mut stdout) = child.stdout.take() {
                let mut buf = vec![0u8; 8192];
                match stdout.read(&mut buf) {
                    Ok(0) => println!("No response"),
                    Ok(n) => println!("Response: {}", String::from_utf8_lossy(&buf[..n]).trim_end_matches('\0')),
                    Err(e) => println!("Read error: {e}"),
                }
            }
        }
        Err(e) => println!("Wait error: {e}"),
    }
    
    child.kill().ok();
    let _ = std::fs::remove_dir_all(&user_data);
}
