# Stage 3 (wreq 替换 reqwest) SDD Progress Ledger

**Plan:** docs/superpowers/plans/2026-07-21-stage3-wreq-migration.md
**Base commit:** 1ffe029 (stage 2 完成)
**Branch:** master
**Started:** 2026-07-21

## Tasks

- [x] Task 1: 更新 Cargo.toml 依赖（reqwest → wreq）
- [x] Task 2: 重写 src/fetch/mod.rs（reqwest → wreq）
- [x] Task 3: 新增 TLS 指纹模拟配置（emulation + header_order）
- [x] Task 4: 更新 src/fetch/proxy.rs 注释
- [x] Task 5: 新建 tests/fetch_test.rs（emulation 配置 + builder + 兼容性测试）
- [x] Task 6: 端到端集成测试与 stage 3 完成验证
- [x] Final whole-branch review (READY_FOR_COMPLETION, 无 Critical/Important; 3 Minor 非阻塞)

## Completion Log

Task 1: complete (commits 1ffe029..da7a187, review APPROVED; Cargo.toml reqwest→wreq 6.0.0-rc + wreq-util 3.0.0-rc, BoringSSL 编译 3m45s 通过; Minor: 临时日志文件留待 Task 6 清理)
Task 2: complete (commits da7a187..7339e4e, review APPROVED; fetch/mod.rs 机械替换 reqwest→wreq, 15+/15-; API 修正: Response::url()→uri() (wreq 6 实际 API), Clone derive 无需手动 impl; 35 lib tests pass)
Task 3: complete (commits 7339e4e..85f8e98, review APPROVED; Config 新增 emulation: Option<Profile> + header_order: Option<Vec<HeaderName>>; API 偏差: wreq_util::Emulation 是 struct 无 Debug→改用 Profile enum; wreq 6.0.0-rc.29 无 headers_order 方法→字段保留但不应用(3 处注释说明); Task 5/6 测试需用 Profile 替代 Emulation; Important: header_order no-op 是 plan 强制要求+Task 5 依赖)
Task 4: complete (commits 85f8e98..b8ccfa9, skipped review (trivial 1-line comment change); proxy.rs 注释 reqwest→wreq, 4 tests pass)
Task 5: complete (commits b8ccfa9..29e6f34, review APPROVED; 新建 tests/fetch_test.rs 7 测试 (config_default/builder_emulation_override/builder_no_emulation/builder_header_order/builder_chain/client_build_with_emulation/client_build_with_no_emulation); src/fetch/mod.rs 新增 #[doc(hidden)] config_ref() 方法; 用 Profile 而非 Emulation; 35 lib + 7 fetch_test = 42 passed)
Task 6: complete (commits 29e6f34..cddb5ca, review APPROVED; tests/integration.rs 追加 mod fetch_test (3 测试: builder_with_emulation_builds/default_config_has_emulation/builder_no_emulation_builds); 32 insertions; 79 total tests pass; 无 reqwest 残留)
