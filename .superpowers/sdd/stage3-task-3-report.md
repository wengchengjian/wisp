# Stage 3 Task 3 报告：TLS 指纹模拟配置（emulation + header_order）

## Status: DONE_WITH_CONCERNS

任务核心目标（TLS 指纹模拟）已实现并编译/测试通过，但 wreq 6.0.0-rc.29 实际 API 与 plan 描述存在两处差异，已做适配（详见 Concerns）。

---

## 已实现内容

### 文件：`f:\project\wisp\src\fetch\mod.rs`

#### 1. 导入（line 11-12）
```rust
use wreq::header::HeaderName;
use wreq_util::Profile;
```
**注：** plan 要求 `use wreq_util::Emulation;`，但实际 `wreq_util::Emulation` 是 struct 且未实现 `Debug`，无法满足 `Config` 的 `#[derive(Debug, Clone)]`。改用 `wreq_util::Profile`（enum，derive `Debug + Clone + Copy + PartialEq + Eq`），且 `Profile` 同样实现 `wreq::IntoEmulation` trait。

#### 2. Config 新增字段（line 25-28）
```rust
/// 浏览器 TLS 指纹模拟（默认 Chrome136，覆盖最广）
pub emulation: Option<Profile>,
/// 自定义 header 顺序（wreq 6.0.0-rc.29 已移除 headers_order 方法，字段保留供未来扩展）
pub header_order: Option<Vec<HeaderName>>,
```

#### 3. Config::default()（line 39-41）
```rust
// 默认 Chrome 136 指纹（覆盖最广）
emulation: Some(Profile::Chrome136),
header_order: None,
```

#### 4. ClientBuilder 新增方法（line 59-75）
```rust
pub fn emulation(mut self, emu: Profile) -> Self { ... }
pub fn no_emulation(mut self) -> Self { ... }
pub fn header_order(mut self, order: Vec<HeaderName>) -> Self { ... }
```

#### 5. build() 应用 emulation（line 91-95）
```rust
// 应用 TLS 指纹模拟（wreq 文档说明会覆盖现有 TLS/HTTP2 配置）
if let Some(emu) = self.config.emulation {
    builder = builder.emulation(emu);
}
// 注：wreq 6.0.0-rc.29 已移除 headers_order 方法，header_order 字段暂不应用
```

`emulation()` 调用位置：在 `redirect()` / `tls_cert_verification()` / `user_agent()` / `proxy()` 之后。wreq 文档说明 emulation 会覆盖现有 TLS/HTTP2 配置，与 redirect/timeout/proxy 不冲突，无需重排。

---

## 编译结果

```
$ cargo check
warning: `wisp` (lib) generated 4 warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 6.26s
```
Exit code: 0（成功）

4 个 warning 均为项目原有问题（与本次改动无关）：
- `unused_variables: opts` in scraper/mod.rs
- `field headless is never read` in page/mod.rs
- `methods wait_js_challenge and wait_managed are never used` in challenge/mod.rs
- `unused import: std::os::windows::process::CommandExt` in browser/mod.rs

无 `header_order` 相关 dead_code warning（Config 字段为 pub，外部可访问）。

---

## 测试结果

```
$ cargo test --lib
running 35 tests
... (全部通过)
test result: ok. 35 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```
**35 passed, 0 failed** — 与 plan 预期一致，无回归。

---

## Commits

- **Hash:** `85f8e989a8938fba839e4b01c4ce88ef384b8e1e`
- **Message:** `feat: Config 新增 emulation + header_order 字段（默认 Chrome136 TLS 指纹）`
- **改动:** 1 file changed, 32 insertions(+)（仅 `src/fetch/mod.rs`）

---

## Concerns（API 差异）

### 1. `Emulation` → `Profile` 类型替换（重大差异）

**Plan 描述：**
> `use wreq_util::Emulation;`，`emulation: Option<Emulation>`，`Emulation::Chrome136`

**实际 wreq-util 3.0.0-rc.14 API：**
- `wreq_util::Emulation` 是 **struct**（derive `Default, Clone, TypedBuilder`），**未实现 `Debug`**
- `wreq_util::Profile` 是 **enum**（derive `Clone, Copy, Hash, Debug, PartialEq, Eq`），变体包括 `Chrome136`、`Firefox128`、`Safari18` 等
- `Profile` 通过 `impl wreq::IntoEmulation for Profile` 实现 `IntoEmulation` trait，可直接传给 `wreq::ClientBuilder::emulation<T: IntoEmulation>(self, emulation: T)`
- `Emulation` struct 也实现 `IntoEmulation`，但因无 `Debug` 无法用于 `#[derive(Debug, Clone)]` 的 Config

**适配方案：** 改用 `Profile` 类型。这影响 Task 5/6 测试中 `use wreq_util::Emulation; Emulation::Firefox128` 等用法，Task 5/6 需相应改为 `use wreq_util::Profile; Profile::Firefox128`。

### 2. `headers_order` 方法不存在于 wreq 6.0.0-rc.29（重大差异）

**Plan 描述：**
> `wreq::ClientBuilder::headers_order(self, order: impl Into<Cow<'static, [HeaderName]>>)` — accepts `Vec<HeaderName>`

**实际 wreq 6.0.0-rc.29 API：**
- `ClientBuilder` **没有** `headers_order` 方法
- 最接近的是 `pub fn orig_headers(mut self, orig_headers: OrigHeaderMap) -> ClientBuilder`（line 742），但 `OrigHeaderMap` 是 `HeaderMap<HeaderCaseName>` 类型，用于保留 header 名大小写，**不是** header 顺序控制
- `headers_order` 方法存在于 wreq **5.3.0**（`src/client/http.rs:460`），在 6.0.0-rc.29 中已被移除/重构

**适配方案：**
- 保留 `header_order: Option<Vec<HeaderName>>` 字段（plan 要求 + Task 5 测试需要 `builder.config_ref().header_order`）
- 保留 `ClientBuilder::header_order(order: Vec<HeaderName>)` 方法（设置 config 字段）
- `build()` 中**不调用** `headers_order`（方法不存在），加注释说明
- 字段虽不被 build() 读取，但因 Config 字段是 pub，外部可访问，无 dead_code warning

**影响：** `header_order` 配置当前实际不生效。如需 header 顺序控制，未来需调研 wreq 6.0 的 `orig_headers(OrigHeaderMap)` 或其他机制。Task 5 测试 `test_builder_header_order` 仍能通过（只验证字段值，不验证实际生效）。

### 3. emulation 调用顺序

Plan 提到若 `cargo check` 报冲突需重排 emulation 位置。实际编译无冲突，emulation 放在 redirect/timeout/proxy/tls_cert_verification 之后正常工作，无需重排。

---

## 总结

任务核心目标（默认 Chrome136 TLS 指纹模拟）已实现，35 个 lib 测试通过，编译无 error。两处 API 差异已适配：
1. `Emulation` → `Profile`（因 Debug trait 缺失）
2. `headers_order` 方法不存在，字段保留但不应用

Task 5/6 的测试代码需相应调整：`Emulation` → `Profile` 类型。
