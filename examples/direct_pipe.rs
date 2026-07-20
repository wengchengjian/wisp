use std::process::{Command, Stdio};
use std::io::{Read, Write};

fn main() {
    let helper = std::env::temp_dir().join("patchright-helper.js");
    let content = std::fs::read_to_string("src/helper/patchright-helper.js").unwrap();
    std::fs::write(&helper, &content).unwrap();

    let chrome = r"C:\Program Files\Google\Chrome\Application\chrome.exe";
    let ud = std::env::temp_dir().join(format!("pr-direct-{}", std::process::id()));

    let mut child = Command::new("node")
        .arg(helper.to_str().unwrap())
        .arg(chrome)
        .arg("--remote-debugging-pipe")
        .arg("--headless=new")
        .arg("--no-first-run")
        .arg("--no-sandbox")
        .arg("--disable-background-networking")
        .arg(format!("--user-data-dir={}", ud.display()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    println!("Helper spawned, PID: {:?}", child.id());
    std::thread::sleep(std::time::Duration::from_secs(3));

    match child.try_wait() {
        Ok(Some(s)) => { println!("Helper EXITED: {s}"); return; }
        Ok(None) => println!("Helper ALIVE!"),
        Err(e) => { println!("ERR: {e}"); return; }
    }

    // Send CDP command via stdin (null-byte delimited)
    let cmd = r#"{"id":1,"method":"Browser.getVersion","params":{}}"#;
    let mut msg = cmd.as_bytes().to_vec();
    msg.push(0);

    let stdin = child.stdin.as_mut().unwrap();
    match stdin.write_all(&msg) {
        Ok(()) => { stdin.flush().unwrap(); println!("Sent CDP command!"); }
        Err(e) => { println!("Write error: {e}"); return; }
    }

    // Read response from stdout
    std::thread::sleep(std::time::Duration::from_secs(2));
    let stdout = child.stdout.as_mut().unwrap();
    let mut buf = vec![0u8; 16384];
    match stdout.read(&mut buf) {
        Ok(0) => println!("No response (EOF)"),
        Ok(n) => println!("RESPONSE: {}", String::from_utf8_lossy(&buf[..n]).trim_end_matches('\0')),
        Err(e) => println!("Read error: {e}"),
    }

    child.kill().ok();
    let _ = std::fs::remove_dir_all(&ud);
}
