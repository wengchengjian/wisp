use std::io::{Read, Write};
use std::ptr;
use std::os::windows::io::FromRawHandle;

type HANDLE = *mut std::ffi::c_void;
type BOOL = i32;
type DWORD = u32;
type LPVOID = *mut std::ffi::c_void;

#[repr(C)]
struct SECURITY_ATTRIBUTES { n_length: DWORD, lp_security_descriptor: LPVOID, b_inherit_handle: BOOL }

#[repr(C)]
struct STARTUPINFOW {
    cb: DWORD, lp_reserved: *mut u16, lp_desktop: *mut u16, lp_title: *mut u16,
    dw_x: DWORD, dw_y: DWORD, dw_x_size: DWORD, dw_y_size: DWORD,
    dw_x_count_chars: DWORD, dw_y_count_chars: DWORD, dw_fill_attribute: DWORD,
    dw_flags: DWORD, w_show_window: u16, cb_reserved2: u16,
    lp_reserved2: *mut u8, h_std_input: HANDLE, h_std_output: HANDLE, h_std_error: HANDLE,
}

#[repr(C)]
struct PROCESS_INFORMATION { h_process: HANDLE, h_thread: HANDLE, dw_process_id: DWORD, dw_thread_id: DWORD }

extern "system" {
    fn CreatePipe(h: *mut HANDLE, w: *mut HANDLE, sa: *mut SECURITY_ATTRIBUTES, size: DWORD) -> BOOL;
    fn CreateProcessW(app: *const u16, cmd: *mut u16, pa: *mut SECURITY_ATTRIBUTES, ta: *mut SECURITY_ATTRIBUTES, inherit: BOOL, flags: DWORD, env: LPVOID, dir: *const u16, si: *mut STARTUPINFOW, pi: *mut PROCESS_INFORMATION) -> BOOL;
    fn SetHandleInformation(h: HANDLE, mask: DWORD, flags: DWORD) -> BOOL;
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn TerminateProcess(h: HANDLE, code: u32) -> BOOL;
    fn WaitForSingleObject(h: HANDLE, ms: DWORD) -> DWORD;
}

// CRT functions for fd management
extern "C" {
    fn _open_osfhandle(osfhandle: isize, flags: i32) -> i32;
    fn _dup2(fd1: i32, fd2: i32) -> i32;
    fn _close(fd: i32) -> i32;
}

const STARTF_USESTDHANDLES: DWORD = 0x0100;
const HANDLE_FLAG_INHERIT: DWORD = 1;

fn to_wide(s: &str) -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() }

fn main() {
    let user_data = std::env::temp_dir().join(format!("pr-win-{}", std::process::id()));
    // Use --remote-debugging-pipe WITHOUT value (Chrome uses fd 3 and fd 4)
    let cmd_str = format!(
        r#""C:\Program Files\Google\Chrome\Application\chrome.exe" --remote-debugging-pipe --headless=new --no-first-run --no-sandbox --disable-background-networking --disable-gpu --user-data-dir={}"#,
        user_data.display()
    );

    unsafe {
        let mut sa = SECURITY_ATTRIBUTES { n_length: std::mem::size_of::<SECURITY_ATTRIBUTES>() as DWORD, lp_security_descriptor: ptr::null_mut(), b_inherit_handle: 1 };
        let (mut cmd_read, mut cmd_write) = (ptr::null_mut() as HANDLE, ptr::null_mut() as HANDLE);
        let (mut resp_read, mut resp_write) = (ptr::null_mut() as HANDLE, ptr::null_mut() as HANDLE);
        assert!(CreatePipe(&mut cmd_read, &mut cmd_write, &mut sa, 0) != 0);
        assert!(CreatePipe(&mut resp_read, &mut resp_write, &mut sa, 0) != 0);

        // Make OUR ends non-inheritable
        SetHandleInformation(cmd_write, HANDLE_FLAG_INHERIT, 0);
        SetHandleInformation(resp_read, HANDLE_FLAG_INHERIT, 0);

        // Set up CRT fd 3 and fd 4 for Chrome to inherit
        // fd 3 = cmd_read (Chrome reads commands from here)
        // fd 4 = resp_write (Chrome writes responses here)
        let fd3 = _open_osfhandle(cmd_read as isize, 0); // _O_RDONLY
        let fd4 = _open_osfhandle(resp_write as isize, 1); // _O_WRONLY
        println!("Opened CRT fds: fd3={fd3}, fd4={fd4}");
        
        // Dup them to exact fd numbers 3 and 4
        if fd3 != 3 { _dup2(fd3, 3); _close(fd3); }
        if fd4 != 4 { _dup2(fd4, 4); _close(fd4); }
        println!("Mapped to fd 3 and fd 4");

        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as DWORD;
        // Don't use STARTF_USESTDHANDLES - let CRT handle inheritance work

        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let mut cmd_wide = to_wide(&cmd_str);

        let ok = CreateProcessW(
            ptr::null(), cmd_wide.as_mut_ptr(),
            ptr::null_mut(), ptr::null_mut(),
            1,          // bInheritHandles = TRUE
            0x08000000, // CREATE_NO_WINDOW
            ptr::null_mut(), ptr::null(),
            &mut si, &mut pi,
        );

        if ok == 0 {
            println!("CreateProcess FAILED! Error: {}", std::io::Error::last_os_error());
            return;
        }
        println!("Chrome PID: {}", pi.dw_process_id);

        // Close Chrome's ends in our process (the CRT fds)
        _close(3);
        _close(4);

        // Wait for Chrome
        std::thread::sleep(std::time::Duration::from_secs(3));
        let wait_result = WaitForSingleObject(pi.h_process, 0);
        if wait_result == 0 {
            println!("Chrome EXITED!");
            return;
        }
        println!("Chrome alive! Sending CDP...");

        // Write to cmd_write (Chrome reads from cmd_read = fd3)
        let cmd = r#"{"id":1,"method":"Browser.getVersion","params":{}}"#;
        let mut msg = cmd.as_bytes().to_vec();
        msg.push(0);
        let mut write_file = std::fs::File::from_raw_handle(cmd_write);
        match write_file.write_all(&msg) {
            Ok(()) => { write_file.flush().ok(); println!("Sent!"); }
            Err(e) => { println!("Write error: {e}"); return; }
        }
        std::mem::forget(write_file);

        // Read from resp_read (Chrome writes to resp_write = fd4)
        std::thread::sleep(std::time::Duration::from_secs(2));
        let mut read_file = std::fs::File::from_raw_handle(resp_read);
        let mut buf = vec![0u8; 16384];
        match read_file.read(&mut buf) {
            Ok(0) => println!("No response"),
            Ok(n) => println!("RESPONSE: {}", String::from_utf8_lossy(&buf[..n]).trim_end_matches('\0')),
            Err(e) => println!("Read error: {e}"),
        }
        std::mem::forget(read_file);

        TerminateProcess(pi.h_process, 0);
        CloseHandle(pi.h_process);
        CloseHandle(pi.h_thread);
    }
    let _ = std::fs::remove_dir_all(&user_data);
}
