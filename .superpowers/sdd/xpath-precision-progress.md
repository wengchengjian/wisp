# XPath 回查精度改进 SDD Progress Ledger

**Plan:** docs/superpowers/plans/2026-07-21-xpath-precision-lookup.md
**Base commit:** cddb5ca (stage 3 完成)
**Branch:** master
**Started:** 2026-07-21

## Tasks

- [x] Task 1: 新建 NodeSignature 类型 + from_scraper + from_sxd + 单元测试
- [x] Task 2: 用签名匹配重写 locate_in_sxd（带回退）+ 集成测试
- [x] Task 3: 用签名匹配重写 find_in_scraper（带回退）+ 全测试套件验证 + 增强测试
- [x] Final whole-branch review (READY_FOR_COMPLETION, 无 Critical/Important; 91 tests pass; sibling_indices 偏差经评估必要正确)

## Completion Log

Task 1: complete (commits cddb5ca..40c3646, review APPROVED; 新增 NodeSignature 路径签名类型 (path: Vec<(tag, first_class)>) + from_scraper/from_sxd 双向构造; sxd API 偏差: dom::Parent→dom::ParentOfChild (变体 Element/Root); mod.rs doc 字段改 pub(crate); 4 signature_tests + 39 lib tests pass; 预期 warning: NodeSignature unused 待 Task 2/3 消除)
Task 2: complete (commits 40c3646..8285b87, review APPROVED; locate_in_sxd 升级为签名匹配+回退; 新增 find_in_sxd + dfs_sxd_match; 4 precision tests pass (test_xpath_relative_from_nested_node 真正走慢路径触发 locate_in_sxd); 既有 bug: xpath_to_css 快速路径 parse_tag_attr_value 不校验 ] 后内容, 用 descendant:: 前缀绕过; Minor: 建议增强 test_xpath_relative_from_nested_node 选第二个 li 区分新旧行为, Task 3 追加)
Task 3: complete (commits 8285b87..9c50985, review APPROVED; find_in_scraper 升级为签名匹配+回退; 新增 find_in_scraper 方法; 4 新测试 (3 sxd→scraper + 1 增强测试 test_locate_in_sxd_precision_second_sibling); 关键偏差: 发现原签名设计缺陷 (多个相同 tag+class sibling 无法区分), 自行加 sibling_indices: Vec<usize> 字段修复, 双向对称 (scraper 用 NodeId, sxd 用指针 PartialEq); 91 total tests pass; NodeSignature warning 全消)
