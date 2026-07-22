# Stage 2 Task 2 报告：Document struct + sxd-document 懒加载基础设施

## 实现内容

按 brief 创建 `Document` 共享所有权 HTML 文档容器，为 Task 3 的 Node 重构铺路。

### 1. 新建文件 `src/parser/document.rs`

完全照搬 brief 代码，未做任何调整：

- `pub struct Document`：包含 `pub(crate) html: Arc<Html>` 与私有 `sxd: OnceCell<Package>`
- `Document::from_html(&str) -> Arc<Self>`：用 `scraper::Html::parse_document` 解析后包成 `Arc`
- `Document::sxd_package(&self) -> &Package`：通过 `OnceCell::get_or_init` 懒加载，调用 `build_sxd_from_html`
- `fn build_sxd_from_html(html: &Html) -> Package`：取 `html.html()` 规范化后的字符串喂给 `sxd_document::parser::parse`，失败回退 `Package::new()`

### 2. 修改 `src/parser/mod.rs`

在 `pub mod difflib;` 与 `pub mod adaptive;` 之间插入 `pub mod document;`，保持字母顺序。

## cargo check 结果

```
    Checking wisp v0.1.0 (F:\project\wisp)
    ...
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.84s
```

退出码 0，编译通过。生成的 4 个警告均为预先存在，与本任务无关：

1. `src/browser/mod.rs:55` unused import `CommandExt`
2. `src/scraper/mod.rs:185` unused variable `opts`
3. `src/page/mod.rs:17` field `headless` never read
4. `src/challenge/mod.rs:126,147` 未使用方法 `wait_js_challenge` / `wait_managed`

新增的 `document.rs` 无任何警告。

## 文件变更

- 新增：`src/parser/document.rs`（51 行）
- 修改：`src/parser/mod.rs`（新增 1 行 `pub mod document;`）

## 自审

### sxd-document 0.3.2 API 验证

brief 代码原样编译通过，无需调整。验证项：

- `sxd_document::parser::parse(&str)` 返回 `Result<Package, ...>` ✅
- `Package::new()` 创建空包 ✅
- `OnceCell::get_or_init` 返回 `&T` ✅，与 `sxd_package(&self) -> &Package` 签名匹配
- `scraper::Html::html()` 返回 `String` 借给 `parse(&str)`，Deref 生效 ✅

### 约束遵守

- 仅修改了指定的两个文件 ✅
- 未添加测试 ✅
- 仅跑 `cargo check` ✅
- Commit message 为中文 ✅
- 字段可见性保持 brief 原样：`html` 为 `pub(crate)`，`sxd` 私有 ✅

### 后续注意点（给 Task 3）

- `Document::from_html` 返回 `Arc<Self>`，Node 重构时需以 `Arc<Document>` 持有
- `sxd_package()` 返回 `&Package`，未做锁保护——`OnceCell` 本身线程安全，可多线程并发读，但若 Task 3 需要从 Package 派生可变引用（如 `as_document()` 返回可变句柄），需要在 Node 层做同步
- `build_sxd_from_html` 静默失败（fallback 到 `Package::new()`），后续若需要严格模式可加日志

## Commit

- SHA：`2f3285ee1d6b13ec16ed95d6e5d01b4dbcc882a3`
- 短 SHA：`2f3285e`
- Subject：`feat: 新增 Document struct + sxd-document 懒加载基础设施`
- 变更：2 files changed, 51 insertions(+)
