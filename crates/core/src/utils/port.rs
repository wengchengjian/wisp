//! 端口工具函数。

use std::net::TcpListener;

/// 检查端口是否可用（未被占用）。
///
/// 尝试绑定 `0.0.0.0:port`，成功则端口可用。
pub fn is_port_available(port: u16) -> bool {
    TcpListener::bind(("0.0.0.0", port)).is_ok()
}

/// 从指定端口开始，查找第一个可用端口。
///
/// # Arguments
/// * `start_port` - 起始端口（优先使用）
/// * `max_attempts` - 最大尝试次数（向后搜索范围）
///
/// # Returns
/// * `Some(port)` - 找到的可用端口
/// * `None` - 在范围内未找到可用端口
///
/// # Example
/// ```
/// use wisp_core::utils::port::find_available_port;
///
/// // 从 8080 开始找，最多尝试 100 个端口
/// if let Some(port) = find_available_port(8080, 100) {
///     println!("Using port: {}", port);
/// }
/// ```
pub fn find_available_port(start_port: u16, max_attempts: u16) -> Option<u16> {
    let end_port = start_port.saturating_add(max_attempts);
    (start_port..=end_port).find(|&port| is_port_available(port))
}

/// 获取系统分配的随机可用端口。
///
/// 绑定 `0.0.0.0:0` 让操作系统分配一个空闲端口。
pub fn get_random_port() -> Option<u16> {
    TcpListener::bind(("0.0.0.0", 0))
        .ok()
        .and_then(|listener| listener.local_addr().ok())
        .map(|addr| addr.port())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_port_available() {
        // 端口 0 总是可用的（系统会分配随机端口）
        assert!(is_port_available(0));
    }

    #[test]
    fn test_find_available_port() {
        // 从高位端口开始找，应该能找到
        let port = find_available_port(49152, 100);
        assert!(port.is_some());
        assert!(port.unwrap() >= 49152);
    }

    #[test]
    fn test_get_random_port() {
        let port = get_random_port();
        assert!(port.is_some());
        assert!(port.unwrap() > 0);
    }
}
