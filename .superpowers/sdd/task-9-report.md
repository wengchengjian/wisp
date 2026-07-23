# Task 9 报告：修复 resolve_href 不过滤非 http scheme

## Status
✅ DONE

## Commit
- Hash: `7409460`
- Branch: `fix/code-review-2026-07-23`
- Message:
  ```
  fix(crawl): resolve_href 过滤非 http/https scheme

  - 对 Url::join 结果检查 scheme，拒绝 javascript:/mailto:/data: 等
  - 修复 follow 非法链接产生无效请求的问题
  ```
- Files changed: `src/crawl/mod.rs`（1 file, +24 / -1）

## 变更摘要

### 修改位置
- `src/crawl/mod.rs:173-185`（`resolve_href` 函数；brief 标注 L166-172，实际位置因前面代码行数偏移略变化，功能位置一致）

### 实现差异
修复前：`Url::join` 结果直接 `map(|u| u.to_string())`，对 `javascript:`/`mailto:`/`data:` 等会构造非 http URL 并返回 `Some`。

修复后：
```rust
let joined = base_url.join(href).ok()?;
// 仅接受 http/https 结果（过滤 javascript: mailto: data: 等被 join 构造的非法 URL）
if joined.scheme() == "http" || joined.scheme() == "https" {
    Some(joined.to_string())
} else {
    None
}
```

签名保持私有不变：`fn resolve_href(base: &str, href: &str) -> Option<String>`。

### 测试
新增 `crawl::tests::resolve_href_rejects_non_http_schemes`（src/crawl/mod.rs:461-476），覆盖：
- `https://` / `http://` 绝对 URL 仍通过
- `javascript:void(0)` / `mailto:a@b.com` / `data:text/html,xxx` 拒绝（返回 None）
- 相对链接 `b` 仍正常解析为 `https://example.com/a/b`

## 测试摘要

### TDD 流程
1. **Step 2（测试失败确认）**：
   ```
   running 1 test
   test crawl::tests::resolve_href_rejects_non_http_schemes ... FAILED
   failures:
       crawl::tests::resolve_href_rejects_non_http_schemes
   panicked at src/crawl/mod.rs:467:9:
       javascript: scheme 应被拒绝
   test result: FAILED. 0 passed; 1 failed
   ```
   符合预期：当前 `Url::join` 对 `javascript:` 等返回 `Some`。

2. **Step 4（修复后通过）**：
   - `cargo build`：成功（仅 7 条预存 warning，与本改动无关，如 `unused import: self::stats::SpiderStats`）。
   - `cargo test --lib crawl::tests::resolve_href`：
     ```
     test crawl::tests::resolve_href_rejects_non_http_schemes ... ok
     test result: ok. 1 passed; 0 failed
     ```
   - `cargo test --lib`（全量回归）：
     ```
     test result: ok. 203 passed; 0 failed; 0 ignored; 0 measured
     ```
   无回归。

## Self-Review

- ✅ 签名不变：`fn resolve_href(base: &str, href: &str) -> Option<String>`，仍为私有。
- ✅ 无 unwrap/expect 在可恢复路径（生产代码用 `?` 与 `if`；测试中的 `.unwrap()` 仅在预存 `spawn_html_server` helper 中）。
- ✅ 注释中文，与 brief Step 3 文案一致。
- ✅ TDD：先写测试 → 确认失败 → 实现 → 确认通过。
- ✅ Commit message 与 brief Step 5 完全一致。
- ✅ 仅 commit `src/crawl/mod.rs`，未误带入 .superpowers/sdd 下其他未暂存文件。

## 顾虑

1. **早返回快路径未重新校验**：`href.starts_with("http://")` / `https://` 的早返回分支未走 `Url::parse`，因此类似 `http://` 后跟非法字符的 href 仍可能直接返回未规范化字符串。此为预存行为，brief 明确要求仅修复 join 后的 scheme 过滤，未要求改早返回逻辑——保持向后兼容，不在本 Task 范围内。
2. **行号偏移**：brief 标注 L166-172，实际当前代码位于 L173-185（前面的 `xpath_auto`/`css` 等方法行数累积导致偏移）。功能定位与修改目标完全一致。
3. **预存 warning**：`cargo build` 报 7 条 unused import warning（如 `StreamExt`、`SpiderStats`、`Client` 等），均与本 Task 改动无关，是仓库既有状态。

## 报告路径
`/home/weng/wisp/.superpowers/sdd/task-9-report.md`
