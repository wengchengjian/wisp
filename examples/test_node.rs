use std::process::{Command, Stdio};
use std::io::Read;
fn main() {
    // Test 1: Can we spawn node?
    let out = Command::new("node").arg("-e").arg("console.log('node works')").output().unwrap();
    println!("node test: {}", String::from_utf8_lossy(&out.stdout).trim());
    
    // Test 2: Write helper and spawn it
    let helper = std::env::temp_dir().join("patchright-helper.js");
    let content = std::fs::read_to_string("src/helper/patchright-helper.js").unwrap();
    std::fs::write(&helper, &content).unwrap();
    println!("Helper written to: {}", helper.display());
    println!("Helper size: {} bytes", content.len());
    
    // Test 3: Spawn helper with chrome path
    let chrome = r"C:\Program Files\Google\Chrome\Application\chrome.exe";
    let ud = std::env::temp_dir().join(format!("pr-t-{}", std::process::id()));
    let mut child = Command::new("node")
        .arg(helper.to_str().unwrap())
        .arg(chrome)
        .arg("--remote-debugging-pipe")
        .arg("--headless=new")
        .arg("--no-first-run")
        .arg("--no-sandbox")
        .arg(format!("--user-data-dir={}", ud.display()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    
    std::thread::sleep(std::time::Duration::from_secs(3));
    match child.try_wait() {
        Ok(Some(s)) => println!("Helper EXITED: {s}"),
        Ok(None) => println!("Helper ALIVE!"),
        Err(e) => println!("Error: {e}"),
    }
    child.kill().ok();
    let _ = std::fs::remove_dir_all(&ud);
}
