//! 带 TTL 的 entry 编解码。

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 序列化带 TTL 的 entry：`[expires_at: 8 bytes BE i64][value...]`。
/// `expires_at == 0` 表示永不过期；非零为 Unix 毫秒时间戳。
pub(super) fn pack_with_ttl(value: &[u8], ttl: Option<Duration>) -> Vec<u8> {
    let expires_at: i64 = match ttl {
        None => 0,
        Some(d) => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|n| n.as_millis() as i64)
                .unwrap_or(0);
            now + d.as_millis() as i64
        }
    };
    let mut buf = expires_at.to_be_bytes().to_vec();
    buf.extend_from_slice(value);
    buf
}

/// 反序列化，返回 `(value, expired)`。文件损坏返回 `None`。
pub(super) fn unpack_and_check(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 8 {
        return None;
    }
    let expires_at = i64::from_be_bytes([
        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
    ]);
    if expires_at == 0 {
        return Some(data[8..].to_vec());
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|n| n.as_millis() as i64)
        .unwrap_or(0);
    if now > expires_at {
        return None; // 已过期
    }
    Some(data[8..].to_vec())
}
