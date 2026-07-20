use std::process::{Command, Stdio};
use std::io::{Read, Write};
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::ptr;

extern "system" {
    fn CreatePipe(hRead: *mut RawHandle, hWrite: *mut RawHandle, sa: *mut SECURITY_ATTRIBUTES, size: u32) -> i32;
}

#[repr(C)]
struct SECURITY_ATTRIBUTES { n_length: u32, lp_security_descriptor: *mut std::ffi::c_void, b_inherit_handle: i32 }

fn create_pipe() -> (RawHandle, RawHandle) {
    unsafe {
        let mut sa = SECURITY_ATTRIBUTES { n_length: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lp_security_descriptor: ptr::null_mut(), b_inherit_handle: 1 };
        let (mut r, mut w) = (ptr::null_mut(), ptr::null_mut());
        assert!(CreatePipe(&mut r, &mut w, &mut sa, 0) != 0);
        (r, w)
    }
}

fn main() {
    let chrome = r"C:\Program Files\Google\Chrome\Application\chrome.exe";
    let user_data = std::env::temp_dir().join(format!("pr-dbg4-{}", std::process::id()));
    
    // Pipe 1: we write (cmd_write) -> Chrome reads as stdin (cmd_read)
    // Pipe 2: Chrome writes as stdout (resp_write) -> we read (resp_read)
    let (cmd_read, cmd_write) = create_pipe();
    let (resp_read, resp_write) = create_pipe();
    
    // Wrap Chrome's ends as Stdio so Rust includes them in handle inheritance
    let chrome_stdin = unsafe { Stdio::from_raw_handle(cmd_read) };
    let chrome_stdout = unsafe { Stdio::from_raw_handle(resp_write) };
    
    let mut child = Command::new(chrome)
        .arg("--remote-debugging-pipe")  // No value = use stdin/stdout
        .arg("--headless=new")
        .arg("--no-first-run")
        .arg("--no-sandbox")
        .arg("--disable-background-networking")
        .arg(format!("--user-data-dir={}", user_data.display()))
        .stdin(chrome_stdin)
        .stdout(chrome_stdout)
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn");
    
    println!("PID: {:?}", child.id());
    std::thread::sleep(std::time::Duration::from_secs(2));
    
    match child.try_wait() {
        Ok(Some(s)) => println!("Exited: {s}"),
        Ok(None) => {
            println!("Chrome RUNNING! Sending CDP...");
            let mut write_file = unsafe { std::fs::File::from_raw_handle(cmd_write) };
            let cmd = r#"{"id":1,"method":"Browser.getVersion","params":{}}"#;
            let mut msg = cmd.as_bytes().to_vec();
            msg.push(0);
            write_file.write_all(&msg).expect("write failed");
            write_file.flush().ok();
            println!("Sent command!");
            
            std::thread::sleep(std::time::Duration::from_secs(1));
            let mut read_file = unsafe { std::fs::File::from_raw_handle(resp_read) };
            let mut buf = vec![0u8; 8192];
            match read_file.read(&mut buf) {
                Ok(0) => println!("No response"),
                Ok(n) => println!("RESPONSE: {}", String::from_utf8_lossy(&buf[..n]).trim_end_matches('\0')),
                Err(e) => println!("Read err: {e}"),
            }
        }
        Err(e) => println!("Err: {e}"),
    }
    child.kill().ok();
    let _ = std::fs::remove_dir_all(&user_data);
}
