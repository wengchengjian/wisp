//! 文件路径安全化与计算。

use std::path::{Path, PathBuf};

/// 将任意 key sanitize 为安全文件名组件。
///
/// 替换文件系统非法字符（`/` `\` `:` `*` `?` `"` `<` `>` `|`）为 `_`，
/// 处理 Windows 保留名（CON/PRN/AUX/NUL/COM1-9/LPT1-9）加 `wisp_` 前缀。
/// 截断至 200 字符防止文件名过长。
pub(super) fn sanitize_key(key: &str) -> String {
    let s: String = key
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect();
    let upper = s.to_uppercase();
    let is_reserved = matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    );
    let base = if is_reserved { format!("wisp_{s}") } else { s };
    base.chars().take(200).collect()
}

/// 计算 entry 文件路径（不执行任何 I/O）。
pub(super) fn path_for(root: &Path, namespace: &str, key: &str) -> PathBuf {
    root.join(sanitize_key(namespace)).join(sanitize_key(key))
}
