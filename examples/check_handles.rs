// This program checks what GetStdHandle returns when spawned with Stdio::piped()
use std::os::windows::io::AsRawHandle;

type HANDLE = *mut std::ffi::c_void;
type DWORD = u32;

extern "system" {
    fn GetStdHandle(n: DWORD) -> HANDLE;
    fn GetFileType(h: HANDLE) -> DWORD;
}

const STD_INPUT_HANDLE: DWORD = 0xFFFFFFF6; // -10
const STD_OUTPUT_HANDLE: DWORD = 0xFFFFFFF5; // -11
const FILE_TYPE_PIPE: DWORD = 3;

fn main() {
    unsafe {
        let input = GetStdHandle(STD_INPUT_HANDLE);
        let output = GetStdHandle(STD_OUTPUT_HANDLE);
        let input_type = GetFileType(input);
        let output_type = GetFileType(output);
        
        println!("INPUT handle: {:?}, type: {} (pipe={})", input, input_type, input_type == FILE_TYPE_PIPE);
        println!("OUTPUT handle: {:?}, type: {} (pipe={})", output, output_type, output_type == FILE_TYPE_PIPE);
        
        if input.is_null() || input == usize::MAX as HANDLE {
            println!("INPUT IS INVALID!");
        }
        if output.is_null() || output == usize::MAX as HANDLE {
            println!("OUTPUT IS INVALID!");
        }
    }
}
