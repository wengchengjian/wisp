use std::process::{Command, Stdio};
use std::io::Read;
use std::os::windows::process::CommandExt;
fn main() {
    let mut child = Command::new(r"C:\Users\wengchengjian\.cargo\target\debug\examples\check_handles.exe")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x08000000)
        .spawn().unwrap();
    let mut out = String::new();
    child.stdout.as_mut().unwrap().read_to_string(&mut out).unwrap();
    child.wait().unwrap();
    println!("{out}");
}
