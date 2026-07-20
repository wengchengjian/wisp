use std::process::{Command, Stdio};
use std::io::{Read, Write};
use std::os::windows::process::CommandExt;

fn main() {
    let chrome = r"C:\Program Files\Google\Chrome\Application\chrome.exe";
    let ud = std::env::temp_dir().join(format!("pr-direct2-{}", std::process::id()));
    
    // Spawn Chrome directly with piped stdin/stdout, NO --no-sandbox
    let mut child = Command::new(chrome)
        .arg("--remote-debugging-pipe")
        .arg("--no-first-run")
        .arg("--disable-background-networking")
        .arg("--disable-blink-features=AutomationControlled")
        .arg(format!("--user-data-dir={}", ud.display()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(0x00000010) // CREATE_NEW_CONSOLE
        .spawn()
        .expect("spawn failed");
    
    println!("Chrome PID: {:?}", child.id());
    std::thread::sleep(std::time::Duration::from_secs(3));
    
    match child.try_wait() {
        Ok(Some(s)) => { println!("EXITED: {s}"); return; }
        Ok(None) => println!("ALIVE!"),
        Err(e) => { println!("ERR: {e}"); return; }
    }
    
    // Send CDP via stdin
    let cmd = r#"{"id":1,"method":"Browser.getVersion","params":{}}"#;
    let mut msg = cmd.as_bytes().to_vec();
    msg.push(0);
    let stdin = child.stdin.as_mut().unwrap();
    match stdin.write_all(&msg) {
        Ok(()) => { stdin.flush().unwrap(); println!("SENT via stdin"); }
        Err(e) => { println!("WRITE ERR: {e}"); return; }
    }
    
    std::thread::sleep(std::time::Duration::from_secs(2));
    let stdout = child.stdout.as_mut().unwrap();
    let mut buf = vec![0u8; 16384];
    match stdout.read(&mut buf) {
        Ok(0) => println!("NO RESPONSE (EOF)"),
        Ok(n) => println!("RESPONSE: {}", String::from_utf8_lossy(&buf[..n]).trim_end_matches('\0')),
        Err(e) => println!("READ ERR: {e}"),
    }
    child.kill().ok();
    let _ = std::fs::remove_dir_all(&ud);
}
