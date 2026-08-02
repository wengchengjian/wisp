//! HTTP 响应拦截检测（状态码 / CF 头 / body 特征）。

use std::collections::HashMap;

/// 检测 HTTP 响应是否被反爬拦截。
///
/// 检测信号：
/// - 状态码 403/429/503
/// - 响应体含 Cloudflare 挑战特征
/// - 响应头含 cf-chl-* 标记
pub fn is_blocked_response(status: u16, body: &[u8], headers: &HashMap<String, String>) -> bool {
    blocked_reason(status, body, headers).is_some()
}

/// 单次遍历窗口，按首字节分流检查四个 CF 特征，避免对同一 body 重复扫描。
fn body_marker_at(window: &[u8], i: usize) -> Option<&'static str> {
    match window[i].to_ascii_lowercase() {
        b'j' if starts_with_ci(window, i, b"just a moment") => Some("body:just a moment"),
        b'c' if starts_with_ci(window, i, b"cf-challenge") => Some("body:cf-challenge"),
        b'a' if starts_with_ci(window, i, b"attention required") => Some("body:attention required"),
        b'a' if starts_with_ci(window, i, b"access denied") => Some("body:access denied"),
        _ => None,
    }
}

pub(crate) fn blocked_body_reason(window: &[u8]) -> Option<&'static str> {
    let mut i = 0;
    while i < window.len() {
        if let Some(reason) = body_marker_at(window, i) {
            return Some(reason);
        }
        i += 1;
    }
    None
}

fn starts_with_ci(haystack: &[u8], start: usize, needle: &[u8]) -> bool {
    haystack[start..]
        .get(..needle.len())
        .is_some_and(|s| s.eq_ignore_ascii_case(needle))
}

fn header_starts_with_ci(name: &str, prefix: &str) -> bool {
    name.as_bytes()
        .get(..prefix.len())
        .is_some_and(|s| s.eq_ignore_ascii_case(prefix.as_bytes()))
}

/// 返回拦截原因（用于诊断日志），未拦截则返回 None。
pub fn blocked_reason(
    status: u16,
    body: &[u8],
    headers: &HashMap<String, String>,
) -> Option<&'static str> {
    // 状态码
    if matches!(status, 403 | 429 | 503) {
        return Some("status_code");
    }
    // CF 响应头检查是 O(header 数) 且命中率高的快速路径，放在 body 扫描之前。
    if headers.keys().any(|k| header_starts_with_ci(k, "cf-chl")) {
        return Some("header:cf-chl");
    }
    // CF 特征（即使 200 也可能是挑战页）
    // 注意："challenge-platform" 已移除——它出现在所有 CF 保护页面的正常 HTML 中
    // （<script src="/cdn-cgi/challenge-platform/...">），不是挑战页的标志。
    // 只扫描头部窗口，单次遍历匹配全部特征，避免重复扫描与分配。
    if body.is_empty() {
        return None;
    }
    const BODY_SCAN_LIMIT: usize = 64 * 1024;
    let window = &body[..body.len().min(BODY_SCAN_LIMIT)];
    if let Some(reason) = blocked_body_reason(window) {
        return Some(reason);
    }
    None
}
