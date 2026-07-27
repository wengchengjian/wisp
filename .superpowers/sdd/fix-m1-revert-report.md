# Fix M-1 Revert — Report

## 变更内容

### 1. 回退 M-1 零分配优化

**文件：** `/home/weng/wisp/src/crawl/engine.rs`（L252-L253）

将基于 `mem::take` 的零分配实现回退为原始的 `to_string()` 版本，恢复 `resp.title` 字段在 move 进 `spider.handle(resp)` 之前的语义完整性。

**Before（被回退）：**

```rust
// 仅浏览器模式下有值；HTTP 模式为空字符串。不在此处调用 parse() 以免占用单次解析槽位。
// 用 mem::take 取出 title 避免 String 分配：resp 紧接着被 move 进 spider.handle，
// 若以 &str 借用会触发 borrow checker。title 字段为诊断字段，spider handler 不读取。
let title_owned = resp.title.take();
let title_for_log: &str = title_owned.as_deref().unwrap_or("");
```

**After（原始实现恢复）：**

```rust
// 仅浏览器模式下有值；HTTP 模式为空字符串。不在此处调用 parse() 以免占用单次解析槽位。
let title_for_log = resp.title().unwrap_or_default().to_string();
```

`process_response` 中其余日志改动（`title_for_log` 变量名、warn/info 行的 `title={:?}` 字段）保持不变。M-1 之外的其他修复（I-1 idx 移除、I-3 trace 日志、M-2 注释、M-3 断言、M-5 文档示例、M-6 文档注记）全部保留。

### 2. Plan 文件入 git

**文件：** `docs/superpowers/plans/2026-07-27-engine-ergonomic-helpers.md`

由于 `docs/` 在 `.gitignore` 中，使用 `git add -f` 强制加入暂存区，与 `2026-07-26-arch-refactor-pr*` 等历史 plan 文件保持一致。文件中已包含上一轮 fix 提交写入的「实际成果」与「Deferred 项」小节。

## 测试命令与结果

| 命令 | 结果 |
| --- | --- |
| `cargo test --workspace --lib --bins` | `test result: ok. 291 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.82s` |
| `cargo build --example novel_crawler` | `Finished \`dev\` profile [unoptimized + debuginfo] target(s) in 4.54s` |

## Plan 文件已被 git 跟踪的确认

```
$ git ls-files docs/superpowers/plans/2026-07-27-engine-ergonomic-helpers.md
docs/superpowers/plans/2026-07-27-engine-ergonomic-helpers.md
```

输出该路径即表示文件已被跟踪。

## Commit 信息

- SHA：`8226e1c`
- Subject：`fix: 回退 M-1 零分配优化以保留 resp.title 字段语义，plan 文件入 git`
- Branch：`feat/engine-ergonomic-helpers`
- Parent：`eaa95a2`
- Diff：2 files changed, 893 insertions(+), 4 deletions(-)
  - `src/crawl/engine.rs`：回退 M-1（-4 / +1）
  - `docs/superpowers/plans/2026-07-27-engine-ergonomic-helpers.md`：新增（+892）

## 未纳入本次提交的改动

`.superpowers/sdd/task-3-report.md` 在工作区有未暂存改动，超出本次 fix 任务范围，未触碰。
