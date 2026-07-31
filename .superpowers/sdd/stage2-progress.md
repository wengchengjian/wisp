# Stage 2 (P1 Parser Enhancement) SDD Progress Ledger

**Plan:** docs/superpowers/plans/2026-07-21-stage2-p1-parser-enhancement.md
**Base commit:** 27f134f (stage 1 cleanup)
**Branch:** master
**Started:** 2026-07-21

## Tasks

- [x] Task 1: 新增 sxd-document/sxd-xpath 依赖与 ParseError 变体
- [x] Task 2: 创建 Document struct + sxd 懒加载基础设施
- [x] Task 3: Node 重构为 Arc<Document> + node_id（最高风险）
- [x] Task 4: DOM 导航真实实现 + 测试
- [x] Task 5: sxd-xpath 完整查询集成
- [x] Task 6: XPath 测试
- [x] Task 7: ElementSnapshot::capture 升级用 Node 导航 API
- [x] Task 8: 端到端集成测试与 stage 2 完成验证
- [x] Final whole-branch review

## Completion Log

Task 1: complete (commits 27f134f..0f86b54, review clean)
Task 2: complete (commits 0f86b54..2f3285e, review clean; note: OnceCell 非 Sync，Task 3 可能需改 OnceLock)
Task 3: complete (commits 2f3285e..cfd88b5, review clean after fix; ego-tree dep added, OnceLock applied, from_fragment hybrid for table elements, Send/Sync 留待未来 Mutex 方案)
Task 4: complete (commits cfd88b5..89471d8, review clean; 9 dom_nav tests pass, matches 用 Selector::matches(&ElementRef) API)
Task 5: complete (commits 89471d8..b178d26, review clean; sxd API 调整: 自写 DFS, evaluate 签名, nodeset::Node 枚举统一)
Task 6: complete (commits b178d26..b14e3c7, review clean; 9 xpath tests pass, 慢路径 contains_href 实际走 sxd-xpath 非 fast path)
Task 7: complete (commits b14e3c7..506c069, review APPROVED; capture 重写用 Node 导航 API, 4 helpers 保留供 similarity() 使用; Minor: text_preview 字节长度 vs 字符数系 brief 本身写法，非缺陷)
Task 8: complete (commits 506c069..1ffe029, review APPROVED; 3 端到端集成测试追加到 adaptive_test mod, 全套 73 测试通过; Minor: brief 表述 integration 4 实际为 9, 工具链 diff 文件损坏用 git show 替代)

## Final Whole-Branch Review (2026-07-21)

**Verdict: READY_FOR_COMPLETION**

- Spec 覆盖：2.2 + 2.3 全部完成，2.1 wreq 推迟（已知）
- 跨任务一致性：完美（Document/Node/xpath_full/capture 签名全部匹配）
- Critical/Important：无
- Minor findings（非阻塞，留待未来）：
  - M1: Send/Sync 限制未在源代码文档注释中说明（仅在 task-3-report.md）
  - M2: select() scoped→global 语义变化文档注释提到"Task 4 计划修复"但 Task 4 未修复（应改为"遗留"）
  - M3: 性能对比"首次开销断言"缺失（benchmark 已覆盖大部分）
  - M4: similarity() 仍用旧 helper 重复解析（stage 1 遗留，stage 2 未要求升级）
  - M5: text_preview.len() 字节长度 vs 字符数（brief 本身写法）

**Stage 2 完成。** 9 commits, 10 files, +730/-166, 73 tests passing.
