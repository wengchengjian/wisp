# Task 1: 删除 SessionManager 模块

**Files:**
- Delete: `src/crawl/session.rs`
- Modify: `src/crawl/mod.rs`
- Delete: `tests/session_test.rs`

## Steps

1. 删除 `src/crawl/session.rs` 文件（用 DeleteFile 工具）
2. 删除 `src/crawl/mod.rs` 的 `pub mod session;` 声明，以及任何 `pub use session::{...}` re-export
3. 删除 Spider trait 的 4 个死方法：
   - `fn configure_sessions(&self, _mgr: &mut session::SessionManager) {}`
   - `fn session_for(&self, _req: &SpiderRequest) -> &str { "default" }`
   - `fn concurrent_requests(&self) -> u32 { 8 }`
   - `fn rotate_ua(&self) -> bool { false }`
4. 删除 `tests/session_test.rs` 文件（用 DeleteFile 工具）
5. 用 Grep 搜索 `SessionManager|FetcherType|configure_sessions|session_for|request_with_session|session_id_of` 在 src/ 和 tests/ 中，清理所有残留引用
6. 验证编译：`cargo build --lib`
7. 提交（PowerShell 不支持 heredoc，用多个 -m 参数）：
   ```
   git rm src/crawl/session.rs tests/session_test.rs
   git add src/crawl/mod.rs
   git commit -m "refactor(crawl): 删除 SessionManager 整模块" -m "设计错误：多 fetcher 路由无业界先例，被 auto_rules 取代" -m "删除 Spider trait 4 个死方法"
   ```

## 背景
SessionManager 是设计错误的多 fetcher 路由模块，无业界先例，被 auto_rules 自动模式升级取代。整模块与 Engine 完全断开，纯死代码。
