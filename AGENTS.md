# AGENTS.md

## 测试约定

- 默认测试工具：`cargo-nextest`（已安装 `0.9.140`）。
- 默认完整测试命令：
  ```bash
  cargo nextest run --workspace --all-features
  ```
- doctest 不被 nextest 执行，需要单独运行：
  ```bash
  cargo test --doc
  ```
- 仅在需要 libtest 兼容性、doctest 或指定 `cargo test` 参数时使用 `cargo test`。
- 常规验证不要额外运行完整 `cargo test --workspace --all-features`；普通测试由 nextest 承担，doctest 单独用 `cargo test --doc`。
