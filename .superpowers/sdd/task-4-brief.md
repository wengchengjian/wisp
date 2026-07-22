# Task 4: 清理 dead_code warnings

**Files:**
- Modify: `src/stealth/challenge.rs`
- Modify: `src/browser/page.rs`
- Modify: `src/browser/mod.rs`
- Modify: `src/fetcher/mod.rs`
- Modify: `src/fetcher/session.rs`
- Modify: `src/crawl/engine.rs`

## Steps

1. 先运行 `cargo build --lib 2>&1` 查看当前所有 warnings，确认实际列表
2. 删除 challenge.rs 的 wait_js_challenge / wait_managed 死方法（约 45 行）
3. 删除 Page.headless 字段（字段+构造赋值，参数保留用于逻辑）
4. 删除 browser/mod.rs 的 CommandExt import（如存在）
5. 删除 fetcher/mod.rs 的 unused imports（Value/WispError）
6. 删除 fetcher/session.rs 的 unused mut
7. 修复 engine.rs 的 final_resp 冗余赋值（`= None` → 不初始化）
8. 处理其他发现的 warnings（如 builder_api_test.rs 的 unused imports）
9. 验证零 warnings：`cargo build --lib 2>&1 | Select-String "warning"` 预期 0
10. 验证测试：cargo test --lib + cargo test
11. 提交（多 -m 参数）

## 关键说明
- 之前的重构可能已改变某些 warnings，以实际 cargo build 输出为准
- 只清理 src/ 下的 warnings；tests/ 的 unused imports 如发现也一并清理
- 如果某 warning 的代码正在使用（非死代码），不要删除，记录到 concerns
- engine.rs:148 的行号可能已变化，搜索 `let mut final_resp` 找到实际位置

## 验证
```
cargo build --lib 2>&1 | Select-String "warning"
```
预期：0 warnings（或只剩 GBK 编码文件的预先存在 warning）。

## 提交
```
git add src/stealth/challenge.rs src/browser/page.rs src/browser/mod.rs src/fetcher/mod.rs src/fetcher/session.rs src/crawl/engine.rs
git commit -m "chore: 清理 dead_code warnings" -m "删除 wait_js_challenge/wait_managed 死方法" -m "删除 Page.headless 死字段" -m "删除 unused imports/mut，修复冗余赋值"
```
