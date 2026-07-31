# Task 2: 创建 Document struct + sxd 懒加载基础设施

**Files:**
- Create: `src/parser/document.rs`
- Modify: `src/parser/mod.rs`（声明子模块）

## Step 1: 创建 src/parser/document.rs

```rust
//! Document: 共享所有权的 HTML 文档容器。
//!
//! 包含 scraper::Html（CSS 查询）和懒加载的 sxd-document::Package（XPath 查询）。
//! Node 通过 Arc<Document> 共享文档，select() 返回的 Node 引用同一文档的树中位置。

use std::sync::Arc;
use std::cell::OnceCell;
use scraper::Html;
use sxd_document::Package;

/// 共享的 HTML 文档。scraper 树用于 CSS 查询和 DOM 导航，
/// sxd-document 树懒加载用于 XPath 查询。
pub struct Document {
    /// scraper 解析的 HTML 树（html5ever 容错）
    pub(crate) html: Arc<Html>,
    /// 懒加载的 sxd-document 包（XPath 用）
    sxd: OnceCell<Package>,
}

impl Document {
    /// 从 HTML 字符串创建文档。
    pub fn from_html(html: &str) -> Arc<Self> {
        let parsed = Html::parse_document(html);
        Arc::new(Self {
            html: Arc::new(parsed),
            sxd: OnceCell::new(),
        })
    }

    /// 获取 sxd-document 包（懒加载）。
    ///
    /// 首次调用时用 html5ever 规范化后的 HTML 喂给 sxd_document::parser，
    /// 解决 sxd 对 HTML5 容错弱的问题。后续调用直接返回缓存的 Package。
    pub fn sxd_package(&self) -> &Package {
        self.sxd.get_or_init(|| build_sxd_from_html(&self.html))
    }
}

/// 用 html5ever（scraper 内部）规范化 HTML 后喂给 sxd-document。
///
/// sxd_document::parser 是 XML 解析器，对 HTML5 容错弱：
/// - `<br>`/`<img>` 等空标签需要自闭合
/// - `<script>`/`<style>` 内容会被当文本
/// html5ever 输出的 `html()` 已经规范化处理了这些。
fn build_sxd_from_html(html: &Html) -> Package {
    // html() 返回规范化的 HTML 字串（含 html/head/body 结构）
    let clean_html = html.html();
    sxd_document::parser::parse(&clean_html)
        .unwrap_or_else(|_| Package::new())
}
```

## Step 2: 在 src/parser/mod.rs 声明 document 子模块

在 `pub mod adaptive;` 之前追加：

```rust
pub mod document;
```

当前 src/parser/mod.rs 的开头是：
```rust
pub mod difflib;
pub mod adaptive;
pub mod generate;
```

改为：
```rust
pub mod difflib;
pub mod document;
pub mod adaptive;
pub mod generate;
```

## Step 3: 运行 cargo check 验证编译

Run: `cargo check`
Expected: 编译通过

## Step 4: 提交

```bash
git add src/parser/document.rs src/parser/mod.rs
git commit -m "feat: 新增 Document struct + sxd-document 懒加载基础设施"
```
