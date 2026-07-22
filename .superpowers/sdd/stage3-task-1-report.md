# Stage 3 Task 1 报告：wreq 替换 reqwest（依赖切换）

## Status: DONE

## 任务摘要

将 wisp 项目的 HTTP 客户端依赖从 `reqwest 0.12` 切换到 `wreq 6.0.0-rc` + `wreq-util 3.0.0-rc`，启用 TLS/JA3/JA4 指纹模拟能力。本任务仅修改 `Cargo.toml` 和 `Cargo.lock`，不触碰 `src/` 文件。

## 实施内容

### Cargo.toml 修改（line 29）

**修改前：**
```toml
reqwest = { version = "0.12", features = ["rustls-tls"], default-features = false }
```

**修改后：**
```toml
wreq = "6.0.0-rc"
wreq-util = "3.0.0-rc"
```

文件位置：`f:\project\wisp\Cargo.toml` 第 29-30 行。

### 依赖解析结果

`cargo fetch` 成功锁定 49 个新依赖到最新兼容版本，关键包：

- `wreq v6.0.0-rc.29`
- `wreq-util v3.0.0-rc.14`
- `wreq-proto v0.2.5`
- `wreq-rt v0.2.2-rc.4`
- `btls v0.5.6` / `btls-sys v0.5.6`（BoringSSL 绑定）
- `tokio-btls v0.5.6`
- `http2 v0.5.19`（wreq 用 http2 替代 http 0.2）
- `bindgen v0.72.1`、`cmake v0.1.58`（构建工具）

### cargo tree 验证

运行 `cargo tree --depth 1 | Select-String -Pattern "wreq|reqwest"` 输出：

```
├── wreq v6.0.0-rc.29
└── wreq-util v3.0.0-rc.14
```

✅ 含 wreq + wreq-util，无 reqwest 残留。

## 依赖编译结果

### 命令

```powershell
cargo build --offline -p wreq -p wreq-util
```

使用 `-p wreq -p wreq-util` 仅编译 wreq 相关包，避免 src/fetch/mod.rs 中 `reqwest::*` 残留引用产生的预期编译错误。

### 结果

✅ **成功**，退出码 0

**编译耗时：** 3 分 45 秒（环境评估时为 9m35s，本次仅约一半时间，可能因 target 缓存或并行度差异）。

**编译链路（按顺序）：**
1. `itertools v0.10.5` / `tokio-util v0.7.18` / `futures-util v0.3.33` / `url v2.5.8`
2. `http2 v0.5.19` / `bindgen v0.72.1`（BoringSSL 构建脚本生成绑定）
3. `wreq-proto v0.2.5` / `tower v0.5.3` / `wreq-rt v0.2.2-rc.4`
4. `btls-sys v0.5.6`（**BoringSSL C++ 编译**，最耗时步骤）
5. `btls v0.5.6` / `tokio-btls v0.5.6`
6. `wreq v6.0.0-rc.29` / `wreq-util v3.0.0-rc.14`
7. `Finished dev profile [unoptimized + debuginfo] target(s) in 3m 45s`

✅ btls-sys + BoringSSL 编译成功
✅ wreq + wreq-util 编译成功
✅ 无依赖编译失败

### 环境信息

- 编译工具链：perl 5.42 / nasm 2.16 / cmake 4.3 / go 1.26（BoringSSL 构建所需，前置条件已验证）
- 代理：`127.0.0.1:7897`（用于 `cargo fetch` 拉取 crates.io 索引和源码，编译过程本身使用 `--offline`）

## Commits

| Hash | Message |
|---|---|
| `da7a187` | `build: 替换 reqwest 为 wreq 6.0.0-rc + wreq-util 3.0.0-rc（TLS 指纹模拟依赖）` |

提交内容：`Cargo.toml`（+1/-1 行）+ `Cargo.lock`（+492/-453 行，依赖图重写）。

## 验证清单

- [x] Cargo.toml 已替换 reqwest → wreq + wreq-util
- [x] cargo tree 确认 wreq + wreq-util 在依赖图中
- [x] cargo tree 确认 reqwest 已移除
- [x] btls-sys + BoringSSL 编译成功（3m 45s）
- [x] wreq + wreq-util 编译成功
- [x] 提交仅包含 Cargo.toml + Cargo.lock（未触碰 src/）
- [x] 未触碰 src/ 文件（Task 2 处理）

## Concerns

无阻塞性问题。

**备注（非阻塞）：**

1. **src/fetch/mod.rs 仍引用 reqwest** — 这是预期状态。`cargo build -p wreq -p wreq-util` 跳过了 wisp 自身编译，因此未触发 src/fetch/mod.rs 的 `reqwest::*` 编译错误。Task 2 将重写 src/fetch/mod.rs，把 `reqwest::` 全部替换为 `wreq::`。

2. **代理依赖** — `cargo fetch` 依赖 `127.0.0.1:7897` 代理访问 crates.io（tuna 镜像）。后续 Task 若需重新拉取新依赖仍需代理；编译阶段无需网络。

3. **临时日志文件** — 在项目根目录生成了 `cargo_build_wreq_stage3_task1.log` 和 `cargo_fetch_stage3_task1.log`（Tee-Object 输出），未纳入提交，可在 Task 6 清理阶段删除。

4. **构建时长** — 实际耗时 3m 45s，远低于环境评估的 9m35s 和计划的 10 分钟估计，说明 BoringSSL 缓存有效，后续 Task 的增量编译应更快。
