### Task 5: P1-7 SpiderRequest.meta 跨 checkpoint 持久化

**Files:**
- Modify: `src/crawl/mod.rs:69-95`
- Test: `tests/p1_meta_persistence_test.rs`（新建）

**Interfaces:**
- Produces: `SpiderRequest.meta` 由 `#[serde(skip)]` 改为 `#[serde(with = "meta_serde")]`，使 meta 随 bincode checkpoint 序列化往返。
- Produces: 私有 `meta_serde` 模块（`serialize`/`deserialize` 两个函数，把 `serde_json::Value` 编码为 `Vec<u8>` JSON 字节供 bincode 处理）。

- [ ] **Step 1: 写失败测试 — meta 经 bincode 往返保持一致**

新建 `tests/p1_meta_persistence_test.rs`：

```rust
//! P1-7: SpiderRequest.meta 随 bincode checkpoint 持久化。

use wisp::crawl::SpiderRequest;
use serde_json::json;

#[test]
fn meta_survives_bincode_roundtrip() {
    let req = SpiderRequest::get("https://example.com/page")
        .with_meta(json!({
            "source_page": "https://example.com/list",
            "page_index": 42,
            "tags": ["a", "b"],
            "nested": { "x": 1.5, "y": null }
        }));

    let bytes = bincode::serialize(&req).expect("serialize");
    let restored: SpiderRequest = bincode::deserialize(&bytes).expect("deserialize");

    assert_eq!(restored.url, "https://example.com/page");
    assert_eq!(restored.meta, req.meta, "meta 必须往返保持一致");
    // 抽查嵌套字段
    assert_eq!(restored.meta["page_index"], 42);
    assert_eq!(restored.meta["tags"][1], "b");
    assert_eq!(restored.meta["nested"]["y"], serde_json::Value::Null);
}

#[test]
fn meta_default_null_when_absent() {
    let req = SpiderRequest::get("https://example.com/x");
    let bytes = bincode::serialize(&req).expect("serialize");
    let restored: SpiderRequest = bincode::deserialize(&bytes).expect("deserialize");
    assert_eq!(restored.meta, serde_json::Value::Null);
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --test p1_meta_persistence_test`
Expected: `meta_survives_bincode_roundtrip` 失败 — `restored.meta` 为 `Value::Null`（因 `#[serde(skip)]` 跳过序列化），不等于原 meta。`meta_default_null_when_absent` 可能 PASS（恰好 Value::Null）。

- [ ] **Step 3: 在 mod.rs 添加 meta_serde 模块**

`src/crawl/mod.rs` 在 `pub enum Method` 定义之前（约 line 51 前）插入私有 serde 辅助模块：

```rust
/// 自定义 serde：把 `serde_json::Value` 编码为 `Vec<u8>` JSON 字节，
/// 绕过 bincode 1.x 不支持 `deserialize_any` 的限制，使 meta 随 checkpoint 往返。
mod meta_serde {
    use serde::{Deserializer, Serialize, Serializer};
    use serde_json::Value;

    pub fn serialize<S: Serializer>(v: &Value, s: S) -> Result<S::Ok, S::Error> {
        let bytes = serde_json::to_vec(v).map_err(serde::ser::Error::custom)?;
        bytes.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Value, D::Error> {
        let bytes = Vec::<u8>::deserialize(d)?;
        serde_json::from_slice(&bytes).map_err(serde::de::Error::custom)
    }
}
```

- [ ] **Step 4: 修改 SpiderRequest.meta 的 serde 属性**

`src/crawl/mod.rs:75-83` 当前：

```rust
    // Task 3：必须用 `#[serde(skip)]` 而非 `#[serde(default)]`。
    // `serde_json::Value` 的 Deserialize 依赖 `deserialize_any`，bincode 1.x 不支持；
    // 用 `#[serde(default)]` 会让 `bincode::deserialize::<CrawlState>`（含 SpiderRequest）
    // 在 checkpoint 恢复路径抛 `DeserializeAnyNotSupported`，导致 seen/pending 全部丢失。
    // `#[serde(skip)]` 在序列化与反序列化两端都跳过 meta（用 Value::Null 默认值），
    // 与 Task 9 的既定行为一致（meta 当前不从 checkpoint 读回）。
    // 83cb940 误改为 `#[serde(default)]` 引入回归，此处恢复。
    #[serde(skip)]
    pub meta: Value,
```

替换为（保留约束说明，更新为 with 方案）：

```rust
    // P1-7：用 `#[serde(with = "meta_serde")]` 使 meta 随 bincode checkpoint 往返。
    // bincode 1.x 不支持 `serde_json::Value` 的 `deserialize_any`，
    // 故通过 `meta_serde` 把 Value 编码为 `Vec<u8>` JSON 字节，bincode 可处理。
    #[serde(with = "meta_serde")]
    pub meta: Value,
```

- [ ] **Step 5: 运行测试验证通过**

Run: `cargo test --test p1_meta_persistence_test && cargo test --lib`
Expected: 2 测试 PASS；lib 206 全绿（含 checkpoint 相关测试 save_checkpoint_persists_seen_urls 等）。

- [ ] **Step 6: 运行 checkpoint 集成测试验证不回归**

Run: `cargo test --test cr_fix_engine_test --test engine_infra_test`
Expected: 全绿（checkpoint 恢复路径未因 meta 序列化改变而破坏）。

- [ ] **Step 7: 提交**

```bash
git add src/crawl/mod.rs tests/p1_meta_persistence_test.rs
git commit -m "feat: SpiderRequest.meta 跨 checkpoint 持久化 (P1-7)"
```

---

