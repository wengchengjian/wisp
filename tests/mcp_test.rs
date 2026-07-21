//! MCP server 端到端测试：通过 stdin/stdout 验证 JSON-RPC 协议。

use std::io::Write;
use std::process::Command;
use std::path::PathBuf;

fn wisp_bin() -> Option<PathBuf> {
    // CARGO_BIN_EXE_wisp 由 cargo 在编译 integration test 时自动注入，
    // 指向 wisp bin target 的绝对路径（兼容非默认 target 目录）。
    // 若测试未通过 cargo 运行（手动执行二进制），回退到 target/debug/wisp。
    let p = PathBuf::from(env!("CARGO_BIN_EXE_wisp"));
    if p.exists() { Some(p) } else { None }
}

#[test]
fn test_mcp_tools_list_via_cli() {
    let Some(bin) = wisp_bin() else {
        eprintln!("SKIP: wisp binary not built, run `cargo build` first");
        return;
    };

    // 启动 wisp mcp serve，发 tools/list，验证响应含 6 个工具
    let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
    let mut child = Command::new(&bin)
        .args(["mcp", "serve", "--db", ":memory:"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn wisp");

    {
        let mut stdin = child.stdin.take().expect("failed to open stdin");
        stdin.write_all(request.as_bytes()).expect("write request");
        stdin.write_all(b"\n").expect("write newline");
        // 关闭 stdin 触发 server 退出
        drop(stdin);
    }

    let output = child.wait_with_output().expect("failed to wait");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let resp: serde_json::Value = serde_json::from_str(stdout.lines().next().unwrap_or(""))
        .expect(&format!("invalid json: {}", stdout));

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);
    let tools = resp["result"]["tools"].as_array().expect("tools should be array");
    assert_eq!(tools.len(), 6, "应有 6 个工具: {}", stdout);
}

#[test]
fn test_mcp_extract_css_via_cli() {
    let Some(bin) = wisp_bin() else {
        eprintln!("SKIP: wisp binary not built");
        return;
    };

    let request = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"extract_css","arguments":{"html":"<p>x</p>","selector":"p"}}}"#;
    let mut child = Command::new(&bin)
        .args(["mcp", "serve", "--db", ":memory:"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn");

    {
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(request.as_bytes()).expect("write");
        stdin.write_all(b"\n").expect("newline");
        drop(stdin);
    }

    let output = child.wait_with_output().expect("wait");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let resp: serde_json::Value = serde_json::from_str(stdout.lines().next().unwrap_or(""))
        .expect(&format!("invalid json: {}", stdout));

    assert_eq!(resp["id"], 2);
    let content = resp["result"]["content"][0]["text"]
        .as_str()
        .expect("content text");
    let parsed: serde_json::Value = serde_json::from_str(content).expect("parsed content");
    let texts = parsed["texts"].as_array().expect("texts array");
    assert_eq!(texts.len(), 1);
    assert_eq!(texts[0].as_str().unwrap(), "x");
}

#[test]
fn test_mcp_unknown_method_returns_error() {
    let Some(bin) = wisp_bin() else {
        eprintln!("SKIP: wisp binary not built");
        return;
    };

    let request = r#"{"jsonrpc":"2.0","id":3,"method":"nonexistent/method"}"#;
    let mut child = Command::new(&bin)
        .args(["mcp", "serve", "--db", ":memory:"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");

    {
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(request.as_bytes()).expect("write");
        stdin.write_all(b"\n").expect("newline");
        drop(stdin);
    }

    let output = child.wait_with_output().expect("wait");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let resp: serde_json::Value = serde_json::from_str(stdout.lines().next().unwrap_or(""))
        .expect(&format!("invalid json: {}", stdout));

    assert_eq!(resp["id"], 3);
    assert!(resp.get("error").is_some(), "应返回 error: {}", stdout);
    assert_eq!(resp["error"]["code"], -32601);
}
