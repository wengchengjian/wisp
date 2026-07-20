use std::process::Stdio;
use std::io::{Read, Write};
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x08000000;

fn main() {
    let chrome = r"C:\Program Files\Google\Chrome\Application\chrome.exe";
    let user_data = std::env::temp_dir().join(format!("pr-rust6-{}", std::process::id()));

    let mut child = std::process::Command::new(chrome)
        .arg("--remote-debugging-pipe")
        .arg("--headless=new")
        .arg("--no-first-run")
        .arg("--no-sandbox")
        .arg("--disable-background-networking")
        .arg("--disable-gpu")
        .arg(format!("--user-data-dir={}", user_data.display()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .expect("failed to spawn");

    println!("PID: {:?}", child.id());
    std::thread::sleep(std::time::Duration::from_secs(3));

    match child.try_wait() {
        Ok(Some(s)) => {
            println!("EXITED: {s}");
            if let Some(mut stderr) = child.stderr.take() {
                let mut buf = String::new();
                stderr.read_to_string(&mut buf).ok();
                if !buf.is_empty() { println!("STDERR: {}", buf.lines().next().unwrap_or("")); }
            }
            return;
        }
        Ok(None) => println!("RUNNING!"),
        Err(e) => { println!("ERR: {e}"); return; }
    }

    // Send CDP command via stdin
    let cmd = r#"{"id":1,"method":"Browser.getVersion","params":{}}"#;
    let mut msg = cmd.as_bytes().to_vec();
    msg.push(0);

    if let Some(mut stdin) = child.stdin.take() {
        match stdin.write_all(&msg) {
            Ok(()) => { stdin.flush().ok(); println!("Sent via stdin!"); }
            Err(e) => { println!("stdin write error: {e}"); return; }
        }
    }

    // Read response from stdout
    std::thread::sleep(std::time::Duration::from_secs(2));
    if let Some(mut stdout) = child.stdout.take() {
        let mut buf = vec![0u8; 16384];
        match stdout.read(&mut buf) {
            Ok(0) => println!("No response on stdout"),
            Ok(n) => println!("STDOUT RESPONSE: {}", String::from_utf8_lossy(&buf[..n]).trim_end_matches('\0')),
            Err(e) => println!("stdout read error: {e}"),
        }
    }

    child.kill().ok();
    let _ = std::fs::remove_dir_all(&user_data);
}
