use std::process::{Command, Stdio};
use std::io::{Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
use std::ptr;

// Windows API
extern "system" {
    fn CreatePipe(
        hReadPipe: *mut RawHandle,
        hWritePipe: *mut RawHandle,
        lpPipeAttributes: *mut SECURITY_ATTRIBUTES,
        nSize: u32,
    ) -> i32;
    fn SetHandleInformation(hObject: RawHandle, dwMask: u32, dwFlags: u32) -> i32;
}

#[repr(C)]
struct SECURITY_ATTRIBUTES {
    n_length: u32,
    lp_security_descriptor: *mut std::ffi::c_void,
    b_inherit_handle: i32,
}

const HANDLE_FLAG_INHERIT: u32 = 0x00000001;

fn create_inheritable_pipe() -> (RawHandle, RawHandle) {
    unsafe {
        let mut sa = SECURITY_ATTRIBUTES {
            n_length: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lp_security_descriptor: ptr::null_mut(),
            b_inherit_handle: 1, // TRUE - inheritable
        };
        let mut read_handle: RawHandle = ptr::null_mut();
        let mut write_handle: RawHandle = ptr::null_mut();
        let result = CreatePipe(&mut read_handle, &mut write_handle, &mut sa, 0);
        assert!(result != 0, "CreatePipe failed");
        (read_handle, write_handle)
    }
}

fn main() {
    let chrome = r"C:\Program Files\Google\Chrome\Application\chrome.exe";
    let user_data = std::env::temp_dir().join(format!("pr-dbg3-{}", std::process::id()));
    
    // Create two pipes:
    // Pipe 1: we write -> Chrome reads (commands)
    // Pipe 2: Chrome writes -> we read (responses)
    let (cmd_read, cmd_write) = create_inheritable_pipe();  // Chrome reads from cmd_read
    let (resp_read, resp_write) = create_inheritable_pipe(); // Chrome writes to resp_write
    
    // Pass handle values to Chrome
    let cmd_read_val = cmd_read as usize;
    let resp_write_val = resp_write as usize;
    
    println!("cmd_read handle: {cmd_read_val}, resp_write handle: {resp_write_val}");
    
    let mut child = Command::new(chrome)
        .arg(format!("--remote-debugging-pipe={cmd_read_val},{resp_write_val}"))
        .arg("--headless=new")
        .arg("--no-first-run")
        .arg("--no-sandbox")
        .arg("--disable-background-networking")
        .arg(format!("--user-data-dir={}", user_data.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("failed to spawn");
    
    println!("PID: {:?}", child.id());
    std::thread::sleep(std::time::Duration::from_secs(2));
    
    match child.try_wait() {
        Ok(Some(s)) => println!("Exited: {s}"),
        Ok(None) => {
            println!("Chrome running! Sending CDP command...");
            // Write to cmd_write (Chrome reads from cmd_read)
            let cmd = r#"{"id":1,"method":"Browser.getVersion","params":{}}"#;
            let mut msg = cmd.as_bytes().to_vec();
            msg.push(0);
            
            let mut write_file = unsafe { std::fs::File::from_raw_handle(cmd_write) };
            match write_file.write_all(&msg) {
                Ok(()) => { println!("Sent!"); write_file.flush().ok(); }
                Err(e) => println!("Write error: {e}"),
            }
            std::mem::forget(write_file); // Don't close the handle
            
            std::thread::sleep(std::time::Duration::from_secs(1));
            
            // Read from resp_read (Chrome writes to resp_write)
            let mut read_file = unsafe { std::fs::File::from_raw_handle(resp_read) };
            let mut buf = vec![0u8; 8192];
            match read_file.read(&mut buf) {
                Ok(0) => println!("No response"),
                Ok(n) => println!("Response: {}", String::from_utf8_lossy(&buf[..n]).trim_end_matches('\0')),
                Err(e) => println!("Read error: {e}"),
            }
            std::mem::forget(read_file);
        }
        Err(e) => println!("Wait error: {e}"),
    }
    
    child.kill().ok();
    let _ = std::fs::remove_dir_all(&user_data);
}
