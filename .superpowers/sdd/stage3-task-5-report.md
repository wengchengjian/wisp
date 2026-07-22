# Stage 3 Task 5 报告：新建 tests/fetch_test.rs（emulation 配置 + builder + 兼容性测试）

## Status: DONE

## 概述

完成 Stage 3 Task 5，创建 `tests/fetch_test.rs`（7 个测试）验证 wreq 切换后的 fetch 模块行为：emulation 配置生效、builder 链式调用、Config 默认值、header_order 设置。同时在 `ClientBuilder` 上新增 `#[doc(hidden)] pub fn config_ref(&self) -> &Config` 方法（测试用，不污染公开 API 文档）。

**关键 API 校正：** 已按任务说明将 plan 中的 `wreq_util::Emulation` 替换为 `wreq_util::Profile`（`Profile` 是正确的枚举类型，`Emulation` 是无 Debug 的结构体）。`Profile::Firefox128` 和 `Profile::Safari18` 变体均存在，无需 fallback。

## 实施内容

### 1. 新增 `config_ref` 方法（src/fetch/mod.rs 第 77-81 行）

在 `ClientBuilder` impl 中，`header_order()` 方法之后、`build()` 方法之前新增：

```rust
    /// 获取配置引用（测试用）
    #[doc(hidden)]
    pub fn config_ref(&self) -> &Config {
        &self.config
    }
```

- 位置：`f:\project\wisp\src\fetch\mod.rs` line 77-81
- `#[doc(hidden)]` 隐藏公开 API 文档，避免污染公共接口
- 仅返回 `&Config` 引用，无副作用

### 2. 创建 tests/fetch_test.rs（73 行，7 个测试）

文件路径：`f:\project\wisp\tests\fetch_test.rs`

测试清单：
1. `test_config_default_has_chrome136_emulation` — Config::default 含 Chrome136、header_order=None
2. `test_builder_emulation_override` — `.emulation(Profile::Firefox128)` 覆盖默认值
3. `test_builder_no_emulation` — `.no_emulation()` 清空 emulation
4. `test_builder_header_order` — `.header_order(vec)` 正确设置
5. `test_builder_chain_emulation_and_header_order` — 链式调用：emulation + header_order + timeout + user_agent
6. `test_client_build_with_emulation` — Client::builder().emulation(Chrome136).build() 成功
7. `test_client_build_with_no_emulation` — Client::builder().no_emulation().build() 成功

所有测试不发起实际网络请求（避免环境依赖）。

## 测试结果

### `cargo test --test fetch_test` — 7 passed

```
running 7 tests
test test_builder_emulation_override ... ok
test test_builder_chain_emulation_and_header_order ... ok
test test_builder_no_emulation ... ok
test test_builder_header_order ... ok
test test_config_default_has_chrome136_emulation ... ok
test test_client_build_with_emulation ... ok
test test_client_build_with_no_emulation ... ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

### `cargo test --lib` — 35 passed（无回归）

```
running 35 tests
...
test result: ok. 35 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

lib 编译有 4 个与本 task 无关的预存 warning（browser/scraper/page/challenge 模块的 unused imports/fields/methods），不影响测试结果。

## 提交

- Hash: `29e6f34657d1b7b2c3f66fbc1186a0dd608fc34c`
- Message: `test: 新建 fetch_test 验证 wreq emulation/header_order 配置（7 测试）`
- Files changed: `tests/fetch_test.rs` (新建, 73 行), `src/fetch/mod.rs` (+6 行)
- Total: 2 files changed, 78 insertions(+)

## 验证清单

- [x] Step 1: 创建 `tests/fetch_test.rs`（7 测试，使用 `Profile` 替代 `Emulation`）
- [x] Step 2: 在 `ClientBuilder` 新增 `config_ref` 方法（`#[doc(hidden)]`）
- [x] Step 3: `cargo test --test fetch_test` → 7 passed
- [x] Step 4: `cargo test --lib` → 35 passed（无回归）
- [x] Step 5: git commit（中文 message，符合规范）

## 关键决策

1. **Profile 替代 Emulation：** 按任务说明的 API 校正，所有测试代码使用 `wreq_util::Profile` 枚举（而非 plan 中的 `Emulation`）。`Profile::Firefox128` 和 `Profile::Safari18` 变体存在，无需 fallback。
2. **`#[doc(hidden)]` 应用：** `config_ref` 是测试辅助方法，不应出现在公开 API 文档中，使用 `#[doc(hidden)]` 属性隐藏。
3. **未触碰其他文件：** 仅修改 `src/fetch/mod.rs`（+6 行）和新建 `tests/fetch_test.rs`。`Config` 和 `ClientBuilder` 的 `emulation/no_emulation/header_order` 方法已在 Task 3 完成，本 task 仅消费这些 API。

## Concerns

无。所有测试通过，无回归，API 校正生效。`header_order` 字段在 Task 3 已注明 wreq 6.0.0-rc.29 移除了 `headers_order()` 方法，字段保留供未来扩展，测试仅验证字段值正确设置（不验证是否应用到 wreq builder）—— 这是设计预期，非缺陷。
