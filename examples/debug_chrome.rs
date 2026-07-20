use std::path::PathBuf;
fn main() {
    let paths = [
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    ];
    for p in &paths {
        let path = PathBuf::from(p);
        println!("{} exists: {}", p, path.exists());
    }
    println!("cfg windows: {}", cfg!(target_os = "windows"));
}
