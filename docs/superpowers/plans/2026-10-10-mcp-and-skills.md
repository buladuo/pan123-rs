# pan123-rs MCP 服务与 Claude Skill 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 pan123-rs 增加 `pan123 mcp` 子命令（手写 stdio MCP server，18 个工具，支持进度通知与真实取消），并交付两套 Claude Skill 与安装集成。

**Architecture:** MCP 协议层（`mcp/protocol.rs`）面向 `trait ToolHandler`，完全不认识 SDK；工具层（`mcp/tools.rs`）实现该 trait 并持有 `Pan123Client`。运行时用「读线程 + 每调用一个工作线程 + 专用写线程」以支持并发与取消，取消能力下沉到 SDK 的传输循环。不引入 tokio/rmcp，零新增依赖。

**Tech Stack:** Rust 2024 edition、clap 4.5、serde_json 1.0、std 线程与 mpsc（无 async）

**Spec:** `docs/superpowers/specs/2026-10-10-mcp-and-skills-design.md`

## Global Constraints

- Rust edition `2024`，`rust-version` 未声明；workspace 统一版本 `0.1.0`。
- **新增依赖必须为零。** 不得引入 `tokio`、`rmcp` 或任何新 crate。
- MCP 协议版本：`2025-06-18`（支持列表另含 `2025-03-26`、`2024-11-05`）。
- stdio 分帧：**换行分隔 JSON-RPC 2.0，消息内不得含换行**。
- MCP server 的 **stdout 只允许出现 MCP 消息**；任何日志走 stderr。
- 工具名与参数名统一 snake_case，共 **18 个**工具。
- 所有 id 类参数接受 number 或 string；所有 id 输出一律序列化为 string。
- 并发上限 **4**；进度通知节流为「≥1% 或 ≥500ms」。
- CI 会跑 `cargo fmt --all -- --check` 与 `cargo clippy --all-features -- -D warnings`，每个任务提交前必须两者皆过。
- 测试不得访问外部网络（`api.123278.com` / `user.123pan.cn` 等）。localhost 回环允许。

## Review Focus

以下 5 类输入/失败模式，spec 隐含要求但没有任何任务的测试覆盖；每一条在所属任务中都要有对应测试步骤。

1. **客户端发来畸形或超大单行 JSON**（单行数十 MB、非 JSON、缺 `jsonrpc` 字段）：期望 server 回 `-32700`/`-32600` 后**继续服务**，不 panic、不 OOM、不死循环。
2. **`notifications/cancelled` 指向不存在或已完成的 id**：期望静默忽略，且不影响任何其他在途请求。
3. **工具参数里的 id 是无效值**（不存在、非数字字符串、超出 u64）：期望返回 `isError: true` 并在文本中指明是哪个参数，而不是 panic 或 `-32603`。
4. **传输工具收到相对路径或含 `..` 的路径**：期望明确报错，而不是把文件落到 server 进程工作目录下的意外位置。
5. **客户端在 `tools/call` 执行途中关闭 stdin 退出**：期望 server 干净退出，不 panic、不留下永久阻塞的写线程。

---

## Phase 1：SDK 取消支持

### Task 1: `CancelToken` 与 `Pan123Error::Cancelled`

**Files:**
- Modify: `crates/pan123-sdk/src/error.rs`
- Modify: `crates/pan123-sdk/src/transfer.rs`（新增类型别名）
- Modify: `crates/pan123-sdk/src/lib.rs`（导出）
- Test: 内联 `#[cfg(test)]` 模块（error.rs 与 transfer.rs）

**Interfaces:**
- Consumes: 无
- Produces:
  - `pan123_sdk::transfer::CancelToken` = `Arc<std::sync::atomic::AtomicBool>`
  - `pan123_sdk::Pan123Error::Cancelled`（单元变体）
  - `CancelTokenExt::check(&self) -> pan123_sdk::Result<()>`，未取消返回 `Ok(())`，已取消返回 `Err(Pan123Error::Cancelled)`

- [ ] **Step 1: 写失败测试**

在 `crates/pan123-sdk/src/error.rs` 末尾追加：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn cancelled_is_not_retryable() {
        assert!(!Pan123Error::Cancelled.is_retryable());
    }

    #[test]
    fn cancelled_is_not_auth_error() {
        assert!(!Pan123Error::Cancelled.is_auth_error());
    }

    #[test]
    fn cancelled_is_not_client_error() {
        assert!(!Pan123Error::Cancelled.is_client_error());
    }

    #[test]
    fn token_starts_uncancelled_and_check_passes() {
        let token: crate::transfer::CancelToken = Arc::new(AtomicBool::new(false));
        assert!(token.check().is_ok());
    }

    #[test]
    fn token_check_fails_after_cancel() {
        let token: crate::transfer::CancelToken = Arc::new(AtomicBool::new(false));
        token.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(matches!(token.check(), Err(Pan123Error::Cancelled)));
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p pan123-sdk error::tests -- --nocapture`
Expected: 编译失败，`no variant or associated item named 'Cancelled' found`（以及 `CancelToken` 与 `check` 未定义）。

- [ ] **Step 3: 实现**

`crates/pan123-sdk/src/error.rs`，在 `Pan123Error` 枚举内 `Config` 变体之后加入：

```rust
    #[error("operation cancelled")]
    Cancelled,
```

`is_retryable` / `is_auth_error` / `is_client_error` 三个方法**均不加入 `Cancelled`** —— 保持原样即可，`matches!` 不会命中它。不要为它新增任何分支。

`crates/pan123-sdk/src/transfer.rs` 顶部 import 区补充：

```rust
use std::sync::atomic::{AtomicBool, Ordering};
```

并在 `pub type ProgressCallback = ...;` 下方加入：

```rust
/// 传输取消令牌。`true` 表示已被取消。
///
/// 由调用方创建并持有；SDK 在传输循环中检查它，一旦置位即返回
/// [`Pan123Error::Cancelled`]。CLI 与 MCP 之外的调用方传 `None` 即可，
/// 行为与加入该特性之前完全一致。
pub type CancelToken = Arc<AtomicBool>;

/// 便于在传输循环里做检查点。
pub trait CancelTokenExt {
    fn check(&self) -> crate::Result<()>;
}

impl CancelTokenExt for CancelToken {
    fn check(&self) -> crate::Result<()> {
        if self.load(Ordering::SeqCst) {
            Err(crate::Pan123Error::Cancelled)
        } else {
            Ok(())
        }
    }
}
```

`crates/pan123-sdk/src/lib.rs` 的 `pub use transfer::{...}` 列表里加入 `CancelToken` 与 `CancelTokenExt`。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p pan123-sdk error::tests -- --nocapture`
Expected: 5 passed。

- [ ] **Step 5: 提交**

```bash
git add crates/pan123-sdk/src/error.rs crates/pan123-sdk/src/transfer.rs crates/pan123-sdk/src/lib.rs
git commit -m "feat(sdk): 新增 CancelToken 与 Cancelled 错误，供传输取消使用"
```

---

### Task 2: 可取消的流拷贝并接入下载循环

**Files:**
- Modify: `crates/pan123-sdk/src/client.rs:601-632`（下载读循环）
- Modify: `crates/pan123-sdk/src/transfer.rs`（新增可测试的拷贝函数）
- Test: `crates/pan123-sdk/src/transfer.rs` 内联测试

**Interfaces:**
- Consumes: Task 1 的 `CancelToken`、`CancelTokenExt::check`
- Produces:
  - `pub fn copy_stream_cancellable<R: Read, W: Write>(reader: &mut R, writer: &mut W, cancel: Option<&CancelToken>, on_chunk: impl FnMut(u64)) -> Result<u64>` —— 返回成功写入的总字节数；`cancel` 置位时返回 `Err(Pan123Error::Cancelled)`。每读完一块调用一次 `on_chunk(累计字节)`。

- [ ] **Step 1: 写失败测试**

在 `crates/pan123-sdk/src/transfer.rs` 末尾追加：

```rust
#[cfg(test)]
mod cancel_tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::atomic::Ordering;

    #[test]
    fn copies_all_bytes_when_not_cancelled() {
        let mut reader = Cursor::new(vec![7u8; 4096]);
        let mut writer = Vec::new();
        let mut seen = Vec::new();
        let written = copy_stream_cancellable(&mut reader, &mut writer, None, |total| {
            seen.push(total)
        })
        .expect("copy should succeed");
        assert_eq!(written, 4096);
        assert_eq!(writer.len(), 4096);
        assert_eq!(seen.last().copied(), Some(4096));
    }

    #[test]
    fn returns_cancelled_when_token_already_set() {
        let token: CancelToken = Arc::new(AtomicBool::new(true));
        let mut reader = Cursor::new(vec![7u8; 4096]);
        let mut writer = Vec::new();
        let err = copy_stream_cancellable(&mut reader, &mut writer, Some(&token), |_| {})
            .expect_err("should be cancelled");
        assert!(matches!(err, crate::Pan123Error::Cancelled));
        assert_eq!(writer.len(), 0, "取消后不应写入任何数据");
    }

    #[test]
    fn stops_midway_when_cancelled_during_copy() {
        // 前 1024 字节拷贝完成后置位取消令牌，此后必须停止。
        let token: CancelToken = Arc::new(AtomicBool::new(false));
        let token_for_cb = Arc::clone(&token);
        let mut reader = Cursor::new(vec![7u8; 8192]);
        let mut writer = Vec::new();
        let err = copy_stream_cancellable(&mut reader, &mut writer, Some(&token), move |total| {
            if total >= 1024 {
                token_for_cb.store(true, Ordering::SeqCst);
            }
        })
        .expect_err("should be cancelled midway");
        assert!(matches!(err, crate::Pan123Error::Cancelled));
        assert!(writer.len() < 8192, "应提前停止，实际写入 {}", writer.len());
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p pan123-sdk cancel_tests -- --nocapture`
Expected: 编译失败，`cannot find function 'copy_stream_cancellable'`。

- [ ] **Step 3: 实现**

`crates/pan123-sdk/src/transfer.rs` 文件顶部 import 区补充：

```rust
use std::io::{Read, Write};
```

在 `CancelTokenExt` 实现之后加入：

```rust
/// 可取消的流拷贝。`on_chunk` 每次读取后调用，参数为累计写入字节数。
pub fn copy_stream_cancellable<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    cancel: Option<&CancelToken>,
    mut on_chunk: impl FnMut(u64),
) -> crate::Result<u64> {
    let mut buffer = vec![0u8; 64 * 1024];
    let mut written: u64 = 0;
    loop {
        if let Some(token) = cancel {
            token.check()?;
        }
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read])?;
        written += read as u64;
        on_chunk(written);
    }
    Ok(written)
}
```

在 `crates/pan123-sdk/src/lib.rs` 的 `pub use transfer::{...}` 加入 `copy_stream_cancellable`。

然后替换 `crates/pan123-sdk/src/client.rs` 的下载循环。原代码（约 601-632 行）为：

```rust
                loop {
                    let read = response.read(&mut buffer)?;
                    if read == 0 {
                        break;
                    }
                    output.write_all(&buffer[..read])?;
                    downloaded += read as u64;

                    if downloaded % (1024 * 1024) == 0 {
                        save_resume_meta(
                            &meta_path,
                            &DownloadResumeMeta {
                                url: link.url.clone(),
                                filename: link.filename.clone(),
                                total_bytes,
                                etag: etag.clone(),
                                last_modified: last_modified.clone(),
                                downloaded_bytes: downloaded,
                                created_at: chrono::Utc::now().timestamp(),
                            },
                        )?;
                    }

                    self.emit(
                        &progress,
                        TransferEvent::Progress {
                            id: transfer_id.clone(),
                            kind: TransferKind::Download,
                            bytes: downloaded,
                            total_bytes,
                        },
                    );
                }
```

替换为：

```rust
                let mut buffer = vec![0u8; 64 * 1024];
                downloaded += crate::transfer::copy_stream_cancellable(
                    &mut response,
                    &mut output,
                    options.transfer.cancel.as_ref(),
                    |written| {
                        let absolute = written + start_offset;
                        if absolute % (1024 * 1024) == 0 {
                            let _ = save_resume_meta(
                                &meta_path,
                                &DownloadResumeMeta {
                                    url: link.url.clone(),
                                    filename: link.filename.clone(),
                                    total_bytes,
                                    etag: etag.clone(),
                                    last_modified: last_modified.clone(),
                                    downloaded_bytes: absolute,
                                    created_at: chrono::Utc::now().timestamp(),
                                },
                            );
                        }
                        self.emit(
                            &progress,
                            TransferEvent::Progress {
                                id: transfer_id.clone(),
                                kind: TransferKind::Download,
                                bytes: absolute,
                                total_bytes,
                            },
                        );
                    },
                )?;
```

注意两点：

1. `save_resume_meta` 原先用 `?` 传播错误，但在 `FnMut` 回调里无法早退，改为 `let _ =` 忽略（续传元数据写失败本就不致命，上传路径已有同样处理，见 `client.rs:920`）。
2. `start_offset` 是本次开始前已下载的字节数（即改动前 `downloaded` 的初值）。在调用 `copy_stream_cancellable` 之前插入一行：

```rust
                let start_offset = downloaded;
```

`copy_stream_cancellable` 内部自带缓冲区，因此原循环外那行 `let mut buffer = vec![0u8; ...];`（约 590 行）必须删除，否则是未使用变量、触发 clippy 告警。

`options.transfer.cancel` 字段在 Task 3 引入；本任务先写 `None` 占位，Task 3 再改为上面的形式。**为了保持本任务可独立编译与测试**，此处先写：

```rust
                    None::<&crate::transfer::CancelToken>,
```

并加注释 `// Task 3 接入 options.transfer.cancel`。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p pan123-sdk cancel_tests -- --nocapture`
Expected: 3 passed。

再跑一次全量确认没有回归：
Run: `cargo test -p pan123-sdk`
Expected: 全部通过。

- [ ] **Step 5: 提交**

```bash
git add crates/pan123-sdk/src/transfer.rs crates/pan123-sdk/src/lib.rs crates/pan123-sdk/src/client.rs
git commit -m "feat(sdk): 下载循环改用可取消的流拷贝"
```

---

### Task 3: `TransferOptions.cancel` 与去掉 `Copy`

**Files:**
- Modify: `crates/pan123-sdk/src/transfer.rs`（`TransferOptions` 定义与三个 derive）
- Modify: `crates/pan123-sdk/src/client.rs`（下载传参、上传检查点、`upload_directory_with` 的 clone）
- Modify: `crates/pan123-cli/src/cli.rs:271,324`（结构体字面量）
- Modify: `examples/complete_usage.rs:75,114`（结构体字面量）
- Test: `crates/pan123-sdk/src/transfer.rs` 内联测试

**Interfaces:**
- Consumes: Task 1 的 `CancelToken`，Task 2 的 `copy_stream_cancellable`
- Produces: `TransferOptions { parallelism: usize, retry: RetryPolicy, cancel: Option<CancelToken> }`（`Clone`，**不再是 `Copy`**）；`UploadOptions`、`DownloadOptions` 同样失去 `Copy`

**背景（已核对，务必按其执行）：** `TransferOptions` 当前是 `#[derive(Debug, Clone, Copy)]`。加入 `Option<Arc<AtomicBool>>` 会破坏 `Copy`，连带内嵌它的 `UploadOptions`/`DownloadOptions` 也失去 `Copy`。现有读点（`options.transfer.retry`、`.parallelism`、`options.resume`）读的都是 `Copy` 字段，**不受影响**。唯一真正依赖结构体 `Copy` 的地方是 `client.rs` 的 `upload_directory_with`：`for _ in 0..parallelism` 循环里 `options` 被 `move` 进线程闭包，靠 `Copy` 才能编译。

- [ ] **Step 1: 写失败测试**

在 `crates/pan123-sdk/src/transfer.rs` 的 `cancel_tests` 模块内追加：

```rust
    #[test]
    fn transfer_options_default_has_no_cancel_token() {
        assert!(TransferOptions::default().cancel.is_none());
    }

    #[test]
    fn transfer_options_carries_cancel_token() {
        let token: CancelToken = Arc::new(AtomicBool::new(false));
        let options = TransferOptions {
            cancel: Some(Arc::clone(&token)),
            ..Default::default()
        };
        assert!(Arc::ptr_eq(options.cancel.as_ref().unwrap(), &token));
    }

    #[test]
    fn upload_and_download_options_propagate_cancel() {
        let token: CancelToken = Arc::new(AtomicBool::new(false));
        let transfer = TransferOptions {
            cancel: Some(Arc::clone(&token)),
            ..Default::default()
        };
        let upload = UploadOptions {
            transfer: transfer.clone(),
            resume: true,
        };
        let download = DownloadOptions {
            transfer,
            resume: true,
        };
        assert!(upload.transfer.cancel.is_some());
        assert!(download.transfer.cancel.is_some());
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p pan123-sdk cancel_tests -- --nocapture`
Expected: 编译失败，`struct 'TransferOptions' has no field named 'cancel'`。

- [ ] **Step 3: 实现**

`crates/pan123-sdk/src/transfer.rs`，把 `TransferOptions` 的定义与 derive 改为：

```rust
#[derive(Debug, Clone)]
pub struct TransferOptions {
    pub parallelism: usize,
    pub retry: RetryPolicy,
    /// 取消令牌。`None` 表示不可取消（CLI 现有行为）。
    pub cancel: Option<CancelToken>,
}

impl Default for TransferOptions {
    fn default() -> Self {
        Self {
            parallelism: 1,  // Web端不支持并发，使用顺序上传
            retry: RetryPolicy::default(),
            cancel: None,
        }
    }
}
```

同时把 `UploadOptions` 与 `DownloadOptions` 的 derive 从 `#[derive(Debug, Clone, Copy)]` 改为 `#[derive(Debug, Clone)]`（两者的 `impl Default` 与 `normalized` 方法体无需改动）。

修复编译点：

1. `crates/pan123-sdk/src/client.rs`，`upload_directory_with` 内 `for _ in 0..parallelism {` 循环体开头（在 `let queue = Arc::clone(&queue);` 之前）插入：

```rust
            // UploadOptions 已不再是 Copy，闭包按值捕获，需每轮克隆一份。
            let options = options.clone();
```

2. `crates/pan123-cli/src/cli.rs:271` 的 `UploadOptions { ... }` 与 `:324` 的 `DownloadOptions { ... }` 两个字面量，各在末尾字段后补 `..Default::default()`（两者本就写全了 `transfer` 与 `resume`，补上是为了将来加字段不再破坏）。即：

```rust
                let options = UploadOptions {
                    transfer: build_transfer_options(jobs, retries),
                    resume: !no_resume,
                    ..Default::default()
                };
```

```rust
                    DownloadOptions {
                        transfer: build_transfer_options(None, retries),
                        resume: true,
                        ..Default::default()
                    },
```

3. `examples/complete_usage.rs:75` 与 `:114` 的 `TransferOptions { parallelism, retry }` 字面量，各补 `cancel: None,`。

4. `crates/pan123-sdk/src/client.rs` 下载路径，把 Task 2 留下的占位：

```rust
                    None::<&crate::transfer::CancelToken>,
```

改为：

```rust
                    options.transfer.cancel.as_ref(),
```

并删除那行 `// Task 3 接入 options.transfer.cancel` 注释。

5. `crates/pan123-sdk/src/client.rs` 上传检查点。在 `upload_file_inner` 的分片派发循环内、每次取到 `part_number` 之后（约 1120 行 `if tx.send(...)` 之前）插入：

```rust
                    if let Some(token) = cancel.as_ref() {
                        if token.check().is_err() {
                            let _ = tx.send((part_number, Err(Pan123Error::Cancelled)));
                            break;
                        }
                    }
```

并在主线程收集结果的 `for (part_number, result) in rx` 循环（约 1131 行）内 `match result {` 之前插入：

```rust
            if let Some(token) = cancel.as_ref() {
                if token.check().is_err() {
                    for handle in handles.drain(..) {
                        let _ = handle.join();
                    }
                    return Err(Pan123Error::Cancelled);
                }
            }
```

在 `upload_file_inner` 签名末尾新增一个形参（`client.rs:785-795`），改后为：

```rust
    fn upload_file_inner(
        &self,
        file_path: &Path,
        parent_id: u64,
        duplicate: DuplicateMode,
        retry: RetryPolicy,
        progress: Option<ProgressCallback>,
        transfer_id: &str,
        parallelism: usize,
        resume: bool,
        cancel: Option<CancelToken>,
    ) -> Result<FileInfo> {
```

并在 `upload_file_with` 的调用处（`client.rs:722-732`）末位传入 `options.transfer.cancel.clone()`。注意在 727-731 行逐字段读取 `options` **之前**先把 `cancel` 取出来，避免借用冲突：

```rust
        let cancel = options.transfer.cancel.clone();
        let result = self.upload_file_inner(
            &file_path,
            parent_id,
            duplicate,
            options.transfer.retry,
            progress.clone(),
            &transfer_id,
            options.transfer.parallelism,
            options.resume,
            cancel,
        );
```

`client.rs` 顶部的 `use crate::transfer::{...}` 列表（第 36-37 行）需加入 `CancelToken`。

`upload_directory_with` 内派发给 `upload_file_with` 的是整个 `options`，其中已含 `cancel`，无需额外改动。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p pan123-sdk cancel_tests -- --nocapture`
Expected: 6 passed。

Run: `cargo test --workspace`
Expected: 全部通过（含 examples 能编译）。

Run: `cargo clippy --all-features -- -D warnings`
Expected: 无告警。

- [ ] **Step 5: 提交**

```bash
git add crates/pan123-sdk/src/transfer.rs crates/pan123-sdk/src/client.rs crates/pan123-cli/src/cli.rs examples/complete_usage.rs
git commit -m "feat(sdk): TransferOptions 支持取消令牌，上传分片循环接入检查点"
```

---

## Phase 2：MCP 协议层

### Task 4: JSON-RPC 分帧与消息类型

**Files:**
- Create: `crates/pan123-cli/src/mcp/mod.rs`
- Create: `crates/pan123-cli/src/mcp/protocol.rs`
- Modify: `crates/pan123-cli/src/main.rs`（声明 `mod mcp;`）

**Interfaces:**
- Consumes: 无
- Produces（均在 `crate::mcp::protocol`）：
  - `pub fn parse_line(line: &str) -> std::result::Result<Value, Value>` —— 返回请求值或一个 JSON-RPC 错误响应（已带 `id`，无法解析时为 `Value::Null`）
  - `pub fn success_response(id: Value, result: Value) -> Value`
  - `pub fn error_response(id: Value, code: i64, message: &str) -> Value`
  - 常量 `PARSE_ERROR: i64 = -32700`、`INVALID_REQUEST: i64 = -32600`、`METHOD_NOT_FOUND: i64 = -32601`、`INVALID_PARAMS: i64 = -32602`、`INTERNAL_ERROR: i64 = -32603`

- [ ] **Step 1: 写失败测试**

创建 `crates/pan123-cli/src/mcp/protocol.rs`，先只写测试与空的函数签名：

```rust
use serde_json::{Value, json};

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

pub fn parse_line(_line: &str) -> std::result::Result<Value, Value> {
    unimplemented!()
}

pub fn success_response(_id: Value, _result: Value) -> Value {
    unimplemented!()
}

pub fn error_response(_id: Value, _code: i64, _message: &str) -> Value {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_well_formed_request() {
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        let request = parse_line(line).expect("should parse");
        assert_eq!(request["method"], "ping");
        assert_eq!(request["id"], 1);
    }

    #[test]
    fn rejects_non_json_with_parse_error() {
        let err = parse_line("not json at all").expect_err("should fail");
        assert_eq!(err["error"]["code"], PARSE_ERROR);
        assert_eq!(err["id"], Value::Null);
    }

    #[test]
    fn rejects_json_without_method() {
        let err = parse_line(r#"{"jsonrpc":"2.0","id":7}"#).expect_err("should fail");
        assert_eq!(err["error"]["code"], INVALID_REQUEST);
        assert_eq!(err["id"], 7);
    }

    #[test]
    fn rejects_wrong_jsonrpc_version() {
        let err = parse_line(r#"{"jsonrpc":"1.0","id":3,"method":"ping"}"#).expect_err("should fail");
        assert_eq!(err["error"]["code"], INVALID_REQUEST);
        assert_eq!(err["id"], 3);
    }

    #[test]
    fn rejects_non_object_json() {
        let err = parse_line("[1,2,3]").expect_err("should fail");
        assert_eq!(err["error"]["code"], INVALID_REQUEST);
    }

    #[test]
    fn parses_notification_without_id() {
        let line = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let request = parse_line(line).expect("should parse");
        assert!(request.get("id").is_none());
    }

    #[test]
    fn builds_success_response() {
        let response = success_response(json!(5), json!({"ok": true}));
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 5);
        assert_eq!(response["result"]["ok"], true);
        assert!(response.get("error").is_none());
    }

    #[test]
    fn builds_error_response() {
        let response = error_response(json!(5), METHOD_NOT_FOUND, "nope");
        assert_eq!(response["id"], 5);
        assert_eq!(response["error"]["code"], METHOD_NOT_FOUND);
        assert_eq!(response["error"]["message"], "nope");
        assert!(response.get("result").is_none());
    }
}
```

创建 `crates/pan123-cli/src/mcp/mod.rs`：

```rust
pub mod protocol;
```

在 `crates/pan123-cli/src/main.rs` 顶部 `mod cli;` 之后加入：

```rust
mod mcp;
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p pan123-cli mcp::protocol -- --nocapture`
Expected: 编译通过但测试 panic，`not implemented`。

- [ ] **Step 3: 实现**

把三个 `unimplemented!()` 函数体替换为：

```rust
/// 解析一行 stdin。成功返回请求对象；失败返回一个已构造好的 JSON-RPC 错误响应。
pub fn parse_line(line: &str) -> std::result::Result<Value, Value> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err(error_response(Value::Null, PARSE_ERROR, "empty message"));
    }
    let value: Value = match serde_json::from_str(trimmed) {
        Ok(value) => value,
        Err(err) => {
            return Err(error_response(
                Value::Null,
                PARSE_ERROR,
                &format!("invalid JSON: {err}"),
            ));
        }
    };
    let Some(object) = value.as_object() else {
        return Err(error_response(
            Value::Null,
            INVALID_REQUEST,
            "message must be a JSON object",
        ));
    };
    let id = object.get("id").cloned().unwrap_or(Value::Null);
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(error_response(
            id,
            INVALID_REQUEST,
            "jsonrpc must be \"2.0\"",
        ));
    }
    if object.get("method").and_then(Value::as_str).is_none() {
        return Err(error_response(id, INVALID_REQUEST, "method is required"));
    }
    Ok(value)
}

pub fn success_response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p pan123-cli mcp::protocol -- --nocapture`
Expected: 8 passed。

覆盖 Review Focus 第 1 条：`rejects_non_json_with_parse_error`、`rejects_non_object_json`、`rejects_json_without_method` 保证畸形输入回错误码而不 panic。

- [ ] **Step 5: 提交**

```bash
git add crates/pan123-cli/src/mcp/ crates/pan123-cli/src/main.rs
git commit -m "feat(cli): MCP JSON-RPC 分帧与消息构造"
```

---

### Task 5: `ToolHandler` trait 与请求分发

**Files:**
- Modify: `crates/pan123-cli/src/mcp/protocol.rs`
- Test: 同文件内联测试

**Interfaces:**
- Consumes: Task 4 的 `parse_line` / 响应构造与错误码常量
- Produces（均在 `crate::mcp::protocol`）：
  - `pub struct ToolSpec { pub name: &'static str, pub description: &'static str, pub input_schema: Value }`
  - `pub struct ToolCallOutcome { pub text: String, pub is_error: bool }`
  - `pub struct CallContext { pub progress: ProgressSink, pub cancel: CancelToken }`
  - `pub trait ToolHandler: Send + Sync + 'static { fn list_tools(&self) -> Vec<ToolSpec>; fn call(&self, name: &str, args: &Value, ctx: &CallContext) -> ToolCallOutcome; }`
  - `pub fn handle_request(handler: &dyn ToolHandler, request: &Value, ctx: &CallContext) -> Option<Value>` —— 返回 `None` 表示这是通知（无 `id`），不应回响应
  - 常量 `SUPPORTED_PROTOCOL_VERSIONS: [&str; 3]`、`LATEST_PROTOCOL_VERSION: &str = "2025-06-18"`

本任务只实现 `initialize` / `tools/list` / `tools/call` / `ping` 的分发逻辑；`ProgressSink` 与运行时在 Task 6 实现，本任务先给出最小可编译版本。

- [ ] **Step 1: 写失败测试**

在 `crates/pan123-cli/src/mcp/protocol.rs` 的 `tests` 模块内追加：

```rust
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    struct FakeHandler;

    impl ToolHandler for FakeHandler {
        fn list_tools(&self) -> Vec<ToolSpec> {
            vec![ToolSpec {
                name: "echo",
                description: "returns its input",
                input_schema: json!({
                    "type": "object",
                    "properties": { "text": { "type": "string" } },
                    "required": ["text"]
                }),
            }]
        }

        fn call(&self, name: &str, args: &Value, _ctx: &CallContext) -> ToolCallOutcome {
            if name != "echo" {
                return ToolCallOutcome { text: format!("unknown tool: {name}"), is_error: true };
            }
            match args.get("text").and_then(Value::as_str) {
                Some(text) => ToolCallOutcome { text: text.to_string(), is_error: false },
                None => ToolCallOutcome { text: "text is required".into(), is_error: true },
            }
        }
    }

    fn ctx() -> CallContext {
        CallContext {
            progress: ProgressSink::discarding(),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    fn request(value: Value) -> Value {
        value
    }

    #[test]
    fn initialize_echoes_supported_version() {
        let handler = FakeHandler;
        let req = request(json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name":"x","version":"1"} }
        }));
        let response = handle_request(&handler, &req, &ctx()).expect("must respond");
        assert_eq!(response["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(response["result"]["serverInfo"]["name"], "pan123");
        assert!(response["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn initialize_falls_back_to_latest_version_when_unknown() {
        let handler = FakeHandler;
        let req = request(json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "1999-01-01" }
        }));
        let response = handle_request(&handler, &req, &ctx()).expect("must respond");
        assert_eq!(response["result"]["protocolVersion"], LATEST_PROTOCOL_VERSION);
    }

    #[test]
    fn tools_list_returns_declared_tools() {
        let handler = FakeHandler;
        let req = request(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
        let response = handle_request(&handler, &req, &ctx()).expect("must respond");
        let tools = response["result"]["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "echo");
        assert!(tools[0]["inputSchema"].is_object());
    }

    #[test]
    fn tools_call_wraps_result_as_text_content() {
        let handler = FakeHandler;
        let req = request(json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "echo", "arguments": { "text": "hi" } }
        }));
        let response = handle_request(&handler, &req, &ctx()).expect("must respond");
        assert_eq!(response["result"]["content"][0]["type"], "text");
        assert_eq!(response["result"]["content"][0]["text"], "hi");
        assert_eq!(response["result"]["isError"], false);
    }

    #[test]
    fn tools_call_reports_tool_failure_with_is_error() {
        let handler = FakeHandler;
        let req = request(json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": "echo", "arguments": {} }
        }));
        let response = handle_request(&handler, &req, &ctx()).expect("must respond");
        assert_eq!(response["result"]["isError"], true);
        assert_eq!(response["result"]["content"][0]["text"], "text is required");
        assert!(response.get("error").is_none(), "工具级失败不得用 JSON-RPC error");
    }

    #[test]
    fn unknown_method_returns_method_not_found() {
        let handler = FakeHandler;
        let req = request(json!({ "jsonrpc": "2.0", "id": 5, "method": "resources/list" }));
        let response = handle_request(&handler, &req, &ctx()).expect("must respond");
        assert_eq!(response["error"]["code"], METHOD_NOT_FOUND);
    }

    #[test]
    fn notification_gets_no_response() {
        let handler = FakeHandler;
        let req = request(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        assert!(handle_request(&handler, &req, &ctx()).is_none());
    }

    #[test]
    fn unknown_notification_is_silently_ignored() {
        let handler = FakeHandler;
        let req = request(json!({ "jsonrpc": "2.0", "method": "notifications/whatever" }));
        assert!(handle_request(&handler, &req, &ctx()).is_none());
    }

    #[test]
    fn ping_returns_empty_result() {
        let handler = FakeHandler;
        let req = request(json!({ "jsonrpc": "2.0", "id": 6, "method": "ping" }));
        let response = handle_request(&handler, &req, &ctx()).expect("must respond");
        assert!(response.get("result").is_some());
    }

    #[test]
    fn tools_call_without_name_is_invalid_params() {
        let handler = FakeHandler;
        let req = request(json!({
            "jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {}
        }));
        let response = handle_request(&handler, &req, &ctx()).expect("must respond");
        assert_eq!(response["error"]["code"], INVALID_PARAMS);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p pan123-cli mcp::protocol -- --nocapture`
Expected: 编译失败，`cannot find trait 'ToolHandler'`。

- [ ] **Step 3: 实现**

在 `crates/pan123-cli/src/mcp/protocol.rs` 顶部补充 import：

```rust
use std::sync::Arc;

use pan123_sdk::CancelToken;
```

（`TransferEvent` 只在 `tools.rs` 里用到，本文件不要引入，否则触发 `unused_imports` 导致 clippy 失败。）

并加入类型与分发实现：

```rust
pub const LATEST_PROTOCOL_VERSION: &str = "2025-06-18";
pub const SUPPORTED_PROTOCOL_VERSIONS: [&str; 3] =
    ["2025-06-18", "2025-03-26", "2024-11-05"];

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

pub struct ToolCallOutcome {
    pub text: String,
    pub is_error: bool,
}

pub struct CallContext {
    pub progress: ProgressSink,
    pub cancel: CancelToken,
}

pub trait ToolHandler: Send + Sync + 'static {
    fn list_tools(&self) -> Vec<ToolSpec>;
    fn call(&self, name: &str, args: &Value, ctx: &CallContext) -> ToolCallOutcome;
}

/// 接收请求，返回应当写回的消息。`None` 表示这是通知，不应回复。
pub fn handle_request(
    handler: &dyn ToolHandler,
    request: &Value,
    ctx: &CallContext,
) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    // 通知（无 id）：一律不回复。
    let Some(id) = id else {
        return None;
    };

    let response = match method.as_str() {
        "initialize" => {
            let requested = request
                .get("params")
                .and_then(|params| params.get("protocolVersion"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let negotiated = if SUPPORTED_PROTOCOL_VERSIONS.contains(&requested) {
                requested
            } else {
                LATEST_PROTOCOL_VERSION
            };
            success_response(
                id,
                json!({
                    "protocolVersion": negotiated,
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": "pan123",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            )
        }
        "ping" => success_response(id, json!({})),
        "tools/list" => {
            let tools = handler
                .list_tools()
                .into_iter()
                .map(|spec| {
                    json!({
                        "name": spec.name,
                        "description": spec.description,
                        "inputSchema": spec.input_schema
                    })
                })
                .collect::<Vec<_>>();
            success_response(id, json!({ "tools": tools }))
        }
        "tools/call" => {
            let params = request.get("params").cloned().unwrap_or(Value::Null);
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(error_response(
                    id,
                    INVALID_PARAMS,
                    "tools/call requires params.name",
                ));
            };
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let outcome = handler.call(name, &arguments, ctx);
            success_response(
                id,
                json!({
                    "content": [ { "type": "text", "text": outcome.text } ],
                    "isError": outcome.is_error
                }),
            )
        }
        _ => error_response(
            id,
            METHOD_NOT_FOUND,
            &format!("method not found: {method}"),
        ),
    };
    Some(response)
}
```

`ProgressSink` 与 `TransferEvent` 的 import 在本任务先按下面 Task 6 的最小版本实现，使本文件可编译：

```rust
/// 进度上报句柄。Task 6 补全真正的发送与节流逻辑。
#[derive(Clone)]
pub struct ProgressSink {
    inner: Option<Arc<ProgressChannel>>,
    token: Option<Value>,
}

struct ProgressChannel {
    tx: std::sync::mpsc::Sender<Value>,
    last: std::sync::Mutex<(u64, std::time::Instant)>,
}

impl ProgressSink {
    /// 不发任何通知的哑实现，用于测试与无 progressToken 的场景。
    pub fn discarding() -> Self {
        Self { inner: None, token: None }
    }

    pub fn report(&self, _progress: u64, _total: Option<u64>, _message: Option<&str>) {}
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p pan123-cli mcp::protocol -- --nocapture`
Expected: 18 passed（Task 4 的 8 个 + 本任务 10 个）。

覆盖 Review Focus 第 3 条中「工具失败走 `isError` 而非 JSON-RPC error」：`tools_call_reports_tool_failure_with_is_error`。

- [ ] **Step 5: 提交**

```bash
git add crates/pan123-cli/src/mcp/protocol.rs
git commit -m "feat(cli): MCP ToolHandler trait 与 initialize/tools 分发"
```

---

### Task 6: 运行时——写线程、工作线程、并发上限与取消

**Files:**
- Modify: `crates/pan123-cli/src/mcp/protocol.rs`
- Test: 同文件内联测试

**Interfaces:**
- Consumes: Task 4、Task 5 的全部内容
- Produces:
  - `pub fn run_server<R: BufRead, W: Write + Send + 'static>(handler: Arc<dyn ToolHandler>, reader: R, writer: W) -> std::io::Result<()>`
  - `ProgressSink::new(tx: Sender<Value>, token: Option<Value>) -> Self`，`report` 按「≥1% 或 ≥500ms」节流
  - `pub const MAX_CONCURRENT_CALLS: usize = 4;`

真正的 `main` 接线在 Task 12，本任务用内存 `BufReader`/`Vec<u8>` 驱动测试。

- [ ] **Step 1: 写失败测试**

在 `crates/pan123-cli/src/mcp/protocol.rs` 的 `tests` 模块内追加：

```rust
    use std::io::{BufReader, Cursor};
    use std::sync::Mutex;
    use std::time::Duration;

    /// 记录每次 call 的线程 id，用于断言并发。
    struct RecordingHandler {
        seen: Mutex<Vec<std::thread::ThreadId>>,
        delay_ms: u64,
    }

    impl ToolHandler for RecordingHandler {
        fn list_tools(&self) -> Vec<ToolSpec> {
            vec![ToolSpec {
                name: "slow",
                description: "sleeps",
                input_schema: json!({ "type": "object" }),
            }]
        }

        fn call(&self, _name: &str, _args: &Value, ctx: &CallContext) -> ToolCallOutcome {
            self.seen.lock().unwrap().push(std::thread::current().id());
            let deadline = std::time::Instant::now() + Duration::from_millis(self.delay_ms);
            while std::time::Instant::now() < deadline {
                if ctx.cancel.load(std::sync::atomic::Ordering::SeqCst) {
                    return ToolCallOutcome { text: "cancelled".into(), is_error: true };
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            ToolCallOutcome { text: "done".into(), is_error: false }
        }
    }

    fn run_with_input(handler: Arc<dyn ToolHandler>, input: &str) -> String {
        let reader = BufReader::new(Cursor::new(input.as_bytes().to_vec()));
        let output = Vec::new();
        let sink = Arc::new(Mutex::new(output));
        let sink_writer = SharedVec(Arc::clone(&sink));
        run_server(handler, reader, sink_writer).expect("server should exit cleanly");
        let bytes = sink.lock().unwrap().clone();
        String::from_utf8(bytes).expect("utf8")
    }

    #[derive(Clone)]
    struct SharedVec(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for SharedVec {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }

    fn response_lines(output: &str) -> Vec<Value> {
        output
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).expect("each stdout line must be valid JSON"))
            .collect()
    }

    /// 一次合法的 initialize，供需要「已初始化会话」的测试前置使用。
    const INITIALIZE_LINE: &str =
        r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#;

    /// 前置一次 initialize，返回其后的响应行（已丢弃 initialize 自己的那条）。
    fn run_session(handler: Arc<dyn ToolHandler>, body: &str) -> Vec<Value> {
        let mut input = String::from(INITIALIZE_LINE);
        input.push('\n');
        input.push_str(body);
        let output = run_with_input(handler, &input);
        let mut lines = response_lines(&output);
        assert_eq!(
            lines.first().map(|line| line["id"].clone()),
            Some(json!(0)),
            "首条响应应来自 initialize"
        );
        lines.remove(0);
        lines
    }

    #[test]
    fn eof_produces_no_panic_and_no_output() {
        let handler: Arc<dyn ToolHandler> = Arc::new(FakeHandler);
        let output = run_with_input(handler, "");
        assert!(output.trim().is_empty());
    }

    #[test]
    fn eof_while_call_in_flight_still_writes_its_response() {
        let handler = Arc::new(RecordingHandler {
            seen: Mutex::new(Vec::new()),
            delay_ms: 150,
        });
        // 输入在派发调用后立刻结束：主线程读到 EOF 后必须等在工作线程上，
        // 而不是直接丢弃它——否则客户端会拿到一个永不到来的响应。
        let body = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"slow","arguments":{}}}"#, "\n",
        );
        let lines = run_session(handler, body);
        assert_eq!(lines.len(), 1, "在途调用必须完成并写出响应");
        assert_eq!(lines[0]["id"], 1);
    }

    #[test]
    fn request_before_initialize_is_rejected() {
        let handler: Arc<dyn ToolHandler> = Arc::new(FakeHandler);
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#, "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#, "\n",
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#, "\n",
        );
        let output = run_with_input(handler, input);
        let lines = response_lines(&output);
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0]["error"]["code"], INVALID_REQUEST,
            "initialize 之前的请求必须被拒绝"
        );
        assert_eq!(lines[1]["id"], 2);
        assert!(
            lines[2]["result"]["tools"].is_array(),
            "initialize 之后必须恢复正常服务"
        );
    }

    #[test]
    fn handles_a_full_session_in_order() {
        let handler: Arc<dyn ToolHandler> = Arc::new(FakeHandler);
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#, "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#, "\n",
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"echo","arguments":{"text":"hi"}}}"#, "\n",
        );
        let output = run_with_input(handler, input);
        let lines = response_lines(&output);
        assert_eq!(lines.len(), 3, "通知不应产生响应行");
        assert_eq!(lines[0]["id"], 1);
        assert_eq!(lines[1]["id"], 2);
        assert_eq!(lines[2]["id"], 3);
        assert_eq!(lines[2]["result"]["content"][0]["text"], "hi");
    }

    #[test]
    fn malformed_line_does_not_stop_the_server() {
        let handler: Arc<dyn ToolHandler> = Arc::new(FakeHandler);
        let body = concat!(
            "not json\n",
            r#"{"jsonrpc":"2.0","id":9,"method":"ping"}"#, "\n",
        );
        let lines = run_session(handler, body);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["error"]["code"], PARSE_ERROR);
        assert_eq!(lines[1]["id"], 9, "畸形输入之后仍须继续服务");
        assert!(lines[1]["result"].is_object(), "后续请求应被正常处理");
    }

    #[test]
    fn oversized_single_line_is_rejected_without_panicking() {
        let handler: Arc<dyn ToolHandler> = Arc::new(FakeHandler);
        let mut body = String::from(r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{"x":""#);
        body.push_str(&"a".repeat(2 * 1024 * 1024));
        body.push_str("\"}}\n");
        let lines = run_session(handler, &body);
        assert_eq!(lines.len(), 1, "超大行应被当作一次请求处理");
        assert!(lines[0]["result"].is_object() || lines[0]["error"].is_object());
    }

    #[test]
    fn concurrent_calls_run_on_different_threads() {
        let handler = Arc::new(RecordingHandler {
            seen: Mutex::new(Vec::new()),
            delay_ms: 200,
        });
        let body = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"slow","arguments":{}}}"#, "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"slow","arguments":{}}}"#, "\n",
        );
        let lines = run_session(handler.clone(), body);
        assert_eq!(lines.len(), 2);
        let seen = handler.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_ne!(seen[0], seen[1], "两个调用应在不同线程上执行");
    }

    #[test]
    fn cancelled_call_gets_no_response() {
        let handler = Arc::new(RecordingHandler {
            seen: Mutex::new(Vec::new()),
            delay_ms: 5_000,
        });
        let body = concat!(
            r#"{"jsonrpc":"2.0","id":42,"method":"tools/call","params":{"name":"slow","arguments":{}}}"#, "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":42,"reason":"user"}}"#, "\n",
        );
        let lines = run_session(handler, body);
        assert!(
            lines.is_empty(),
            "被取消的请求不应返回任何响应，实际：{lines:?}"
        );
    }

    #[test]
    fn cancelled_unknown_id_is_ignored() {
        let handler: Arc<dyn ToolHandler> = Arc::new(FakeHandler);
        let body = concat!(
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":9999}}"#, "\n",
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#, "\n",
        );
        let lines = run_session(handler, body);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["id"], 1);
        assert!(lines[0]["result"].is_object(), "后续请求不应受影响");
    }

    #[test]
    fn cancelled_running_call_does_not_block_later_requests() {
        let handler = Arc::new(RecordingHandler {
            seen: Mutex::new(Vec::new()),
            delay_ms: 400,
        });
        let body = concat!(
            r#"{"jsonrpc":"2.0","id":42,"method":"tools/call","params":{"name":"slow","arguments":{}}}"#, "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":42}}"#, "\n",
            r#"{"jsonrpc":"2.0","id":43,"method":"ping"}"#, "\n",
        );
        let lines = run_session(handler, body);
        assert_eq!(lines.len(), 1, "只有 ping 应有响应，被取消的调用不应有");
        assert_eq!(lines[0]["id"], 43);
    }

    #[test]
    fn exceeded_concurrency_returns_is_error() {
        let handler = Arc::new(RecordingHandler {
            seen: Mutex::new(Vec::new()),
            delay_ms: 500,
        });
        let mut body = String::new();
        for id in 1..=6 {
            body.push_str(&format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"tools/call\",\"params\":{{\"name\":\"slow\",\"arguments\":{{}}}}}}\n"
            ));
        }
        let lines = run_session(handler, &body);
        assert_eq!(lines.len(), 6, "每个请求都必须有一条响应");
        let rejected = lines
            .iter()
            .filter(|line| line["result"]["isError"] == json!(true))
            .count();
        assert!(rejected >= 2, "超出并发上限的请求应被拒绝，实际 {rejected}");
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p pan123-cli mcp::protocol::tests -- --nocapture`
Expected: 编译失败，`cannot find function 'run_server'`。

- [ ] **Step 3: 实现**

在 `crates/pan123-cli/src/mcp/protocol.rs` 顶部 import 补充：

```rust
use std::collections::HashMap;
use std::io::BufRead;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
```

加入常量与运行时：

```rust
pub const MAX_CONCURRENT_CALLS: usize = 4;

/// 单行上限，防止畸形客户端用超大行拖垮进程。
const MAX_LINE_BYTES: u64 = 8 * 1024 * 1024;

struct ProgressChannel {
    tx: Sender<Value>,
    last: Mutex<(u64, std::time::Instant)>,
}

impl ProgressSink {
    pub fn new(tx: Sender<Value>, token: Option<Value>) -> Self {
        Self {
            inner: Some(Arc::new(ProgressChannel {
                tx,
                last: Mutex::new((0, std::time::Instant::now())),
            })),
            token,
        }
    }

    /// 按「进度变化 ≥1% 或距上次 ≥500ms」节流后上报。没有 progressToken 时直接丢弃。
    pub fn report(&self, progress: u64, total: Option<u64>, message: Option<&str>) {
        let (Some(channel), Some(token)) = (self.inner.as_ref(), self.token.as_ref()) else {
            return;
        };
        {
            let mut last = match channel.last.lock() {
                Ok(guard) => guard,
                Err(_) => return,
            };
            let step_ok = match total {
                Some(total) if total > 0 => {
                    let last_percent = (*last).0.saturating_mul(100) / total;
                    let now_percent = progress.saturating_mul(100) / total;
                    now_percent > last_percent
                }
                _ => false,
            };
            let time_ok = last.1.elapsed() >= std::time::Duration::from_millis(500);
            if !step_ok && !time_ok && progress != 0 {
                return;
            }
            *last = (progress, std::time::Instant::now());
        }
        let _ = channel.tx.send(json!({
            "jsonrpc": "2.0",
            "method": "notifications/progress",
            "params": {
                "progressToken": token,
                "progress": progress,
                "total": total,
                "message": message
            }
        }));
    }
}

pub fn run_server<R: BufRead, W: Write + Send + 'static>(
    handler: Arc<dyn ToolHandler>,
    mut reader: R,
    writer: W,
) -> std::io::Result<()> {
    let (tx, rx) = mpsc::channel::<Value>();

    // 写线程独占 writer，串行写出每一行。
    let writer_handle = std::thread::spawn(move || {
        let mut writer = writer;
        for message in rx {
            if serde_json::to_writer(&mut writer, &message).is_err() {
                break;
            }
            if writer.write_all(b"\n").is_err() || writer.flush().is_err() {
                break;
            }
        }
    });

    let pending: Arc<Mutex<HashMap<String, CancelToken>>> = Arc::new(Mutex::new(HashMap::new()));
    let active_workers: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
    let mut worker_handles = Vec::new();
    let mut line = String::new();
    let mut initialized = false;

    loop {
        line.clear();
        let read = (&mut reader).take(MAX_LINE_BYTES).read_line(&mut line)?;
        if read == 0 {
            break; // EOF：客户端关闭，干净退出。
        }
        let parse_result = parse_line(&line);
        let request = match parse_result {
            Ok(request) => request,
            Err(error_message) => {
                let _ = tx.send(error_message);
                continue;
            }
        };

        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        // 生命周期状态机：initialize 之前只接受 initialize 本身。
        if method != "initialize" && !initialized {
            if let Some(id) = request.get("id").cloned() {
                let _ = tx.send(error_response(id, INVALID_REQUEST, "必须先调用 initialize"));
            }
            continue; // 无 id 的通知在此静默忽略
        }
        if method == "initialize" {
            initialized = true;
        }

        if method == "notifications/cancelled" {
            let key = request
                .get("params")
                .and_then(|params| params.get("requestId"))
                .map(id_key);
            if let Some(key) = key {
                if let Ok(guard) = pending.lock() {
                    if let Some(token) = guard.get(&key) {
                        token.store(true, Ordering::SeqCst);
                    }
                }
            }
            continue;
        }

        let progress_token = request
            .get("params")
            .and_then(|params| params.get("_meta"))
            .and_then(|meta| meta.get("progressToken"))
            .cloned();
        let write_tx = tx.clone();
        let handler_for_worker = Arc::clone(&handler);
        let is_tool_call = method == "tools/call";
        let request_id = request.get("id").cloned();
        let pending_for_worker = Arc::clone(&pending);
        let active_for_worker = Arc::clone(&active_workers);

        // initialize / tools/list / ping 就地处理；tools/call 派给工作线程。
        let ctx = CallContext {
            progress: ProgressSink::new(write_tx.clone(), progress_token),
            cancel: Arc::new(AtomicBool::new(false)),
        };

        if !is_tool_call {
            if let Some(response) = handle_request(handler_for_worker.as_ref(), &request, &ctx) {
                let _ = write_tx.send(response);
            }
            continue;
        }

        let Some(id) = request_id else {
            continue; // tools/call 作为通知：无 id，忽略。
        };
        let key = id_key(&id);

        if active_for_worker.fetch_add(1, Ordering::SeqCst) >= MAX_CONCURRENT_CALLS {
            active_for_worker.fetch_sub(1, Ordering::SeqCst);
            let _ = write_tx.send(success_response(
                id,
                json!({
                    "content": [{
                        "type": "text",
                        "text": format!("并发上限为 {MAX_CONCURRENT_CALLS}，请稍后重试")
                    }],
                    "isError": true
                }),
            ));
            continue;
        }

        let cancel = ctx.cancel.clone();
        if let Ok(mut guard) = pending.lock() {
            guard.insert(key.clone(), cancel.clone());
        }

        let handle = std::thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    handle_request(handler_for_worker.as_ref(), &request, &ctx)
                }));
                let cancelled = cancel.load(Ordering::SeqCst);
                if !cancelled {
                    let response = match result {
                        Ok(Some(response)) => Some(response),
                        Ok(None) => None,
                        Err(_) => Some(success_response(
                            id,
                            json!({
                                "content": [{ "type": "text", "text": "工具执行时发生内部错误" }],
                                "isError": true
                            }),
                        )),
                    };
                    if let Some(response) = response {
                        let _ = write_tx.send(response);
                    }
                }
                if let Ok(mut guard) = pending_for_worker.lock() {
                    guard.remove(&key);
                }
                active_for_worker.fetch_sub(1, Ordering::SeqCst);
            })
            .expect("failed to spawn tool worker thread");
        worker_handles.push(handle);
    }

    for handle in worker_handles {
        let _ = handle.join();
    }
    drop(tx);
    let _ = writer_handle.join();
    Ok(())
}

/// 请求 id 归一化为字符串键：JSON 数字与字符串形式视为同一个 id。
fn id_key(id: &Value) -> String {
    match id {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}
```

并把 `ProgressSink` 的最小定义改为带 `inner`/`token` 字段的版本（Task 5 的 `discarding()` 保持返回 `inner: None, token: None`）。

删除 Task 5 里那个重复的 `struct ProgressChannel` 与 `impl ProgressSink { pub fn report(...) {} }` 占位，只保留本任务这一份。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p pan123-cli mcp::protocol::tests -- --nocapture`
Expected: 全部通过。整个模块约 2 秒内跑完——`RecordingHandler` 每 5ms 轮询一次取消令牌，被取消的调用会立刻返回，不会跑满 5 秒延时。

覆盖 Review Focus 第 1、2、5 条：
- 第 1 条（畸形/超大输入）：`malformed_line_does_not_stop_the_server`、`oversized_single_line_is_rejected_without_panicking`
- 第 2 条（取消无关 id 不受影响）：`cancelled_unknown_id_is_ignored`、`cancelled_running_call_does_not_block_later_requests`
- 第 5 条（执行中关闭 stdin）：`eof_produces_no_panic_and_no_output`、`eof_while_call_in_flight_still_writes_its_response`

- [ ] **Step 5: 提交**

```bash
git add crates/pan123-cli/src/mcp/protocol.rs
git commit -m "feat(cli): MCP 运行时支持并发、取消与进度节流"
```

---

## Phase 3：MCP 工具层

### Task 7: 18 个工具的 schema 与自检

**Files:**
- Create: `crates/pan123-cli/src/mcp/tools.rs`
- Modify: `crates/pan123-cli/src/mcp/mod.rs`
- Test: `crates/pan123-cli/src/mcp/tools.rs` 内联测试

**Interfaces:**
- Consumes: Task 5 的 `ToolSpec`
- Produces:
  - `pub struct Pan123Tools { pub client: Pan123Client }`
  - `pub fn tool_specs() -> Vec<ToolSpec>`
  - `pub fn id_argument(value: &Value, key: &str) -> Result<u64, String>` —— 接受 number 或 string 两种形式
  - `pub fn ids_argument(value: &Value, key: &str) -> Result<Vec<u64>, String>`

本任务只做 schema 与参数解析辅助函数；`impl ToolHandler` 在 Task 8-9。

- [ ] **Step 1: 写失败测试**

创建 `crates/pan123-cli/src/mcp/tools.rs`：

```rust
use pan123_sdk::Pan123Client;
use serde_json::{Value, json};

use crate::mcp::protocol::ToolSpec;

pub struct Pan123Tools {
    pub client: Pan123Client,
}

pub fn tool_specs() -> Vec<ToolSpec> {
    unimplemented!()
}

pub fn id_argument(_value: &Value, _key: &str) -> Result<u64, String> {
    unimplemented!()
}

pub fn ids_argument(_value: &Value, _key: &str) -> Result<Vec<u64>, String> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn declares_eighteen_tools() {
        assert_eq!(tool_specs().len(), 18);
    }

    #[test]
    fn tool_names_are_unique_and_snake_case() {
        let mut seen = HashSet::new();
        for spec in tool_specs() {
            assert!(
                seen.insert(spec.name),
                "重复的工具名：{}",
                spec.name
            );
            assert!(
                spec.name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "工具名必须是 snake_case：{}",
                spec.name
            );
        }
    }

    #[test]
    fn every_schema_is_an_object_with_properties() {
        for spec in tool_specs() {
            assert_eq!(
                spec.input_schema["type"], "object",
                "{} 的 inputSchema.type 必须是 object",
                spec.name
            );
            assert!(
                spec.input_schema["properties"].is_object(),
                "{} 缺少 properties",
                spec.name
            );
            assert!(
                !spec.description.is_empty(),
                "{} 缺少描述",
                spec.name
            );
        }
    }

    #[test]
    fn required_fields_exist_in_properties() {
        for spec in tool_specs() {
            let properties = spec.input_schema["properties"]
                .as_object()
                .expect("properties object");
            let Some(required) = spec.input_schema.get("required").and_then(Value::as_array)
            else {
                continue;
            };
            for field in required {
                let name = field.as_str().expect("required entries are strings");
                assert!(
                    properties.contains_key(name),
                    "{} 的 required 字段 {name} 不在 properties 中",
                    spec.name
                );
            }
        }
    }

    #[test]
    fn id_fields_accept_number_or_string() {
        assert_eq!(id_argument(&json!(123), "parent_id").unwrap(), 123);
        assert_eq!(id_argument(&json!("123"), "parent_id").unwrap(), 123);
    }

    #[test]
    fn id_argument_reports_the_offending_key() {
        let err = id_argument(&json!("not-a-number"), "parent_id").unwrap_err();
        assert!(err.contains("parent_id"), "错误信息应指明参数名：{err}");
    }

    #[test]
    fn id_argument_rejects_missing_value() {
        let err = id_argument(&json!(null), "file_id").unwrap_err();
        assert!(err.contains("file_id"));
    }

    #[test]
    fn id_argument_rejects_value_beyond_u64() {
        let err = id_argument(&json!("99999999999999999999999"), "file_id").unwrap_err();
        assert!(err.contains("file_id"));
    }

    #[test]
    fn ids_argument_accepts_arrays_of_mixed_forms() {
        let ids = ids_argument(&json!([1, "2", 3]), "file_ids").unwrap();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn ids_argument_rejects_empty_array() {
        let err = ids_argument(&json!([]), "file_ids").unwrap_err();
        assert!(err.contains("file_ids"));
    }

    #[test]
    fn ids_argument_rejects_non_array() {
        let err = ids_argument(&json!(5), "file_ids").unwrap_err();
        assert!(err.contains("file_ids"));
    }
}
```

在 `crates/pan123-cli/src/mcp/mod.rs` 加入：

```rust
pub mod tools;
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p pan123-cli mcp::tools -- --nocapture`
Expected: 编译通过，测试 panic `not implemented`。

- [ ] **Step 3: 实现**

把三个 `unimplemented!()` 替换为：

```rust
pub fn id_argument(value: &Value, key: &str) -> Result<u64, String> {
    match value {
        Value::Number(number) => number
            .as_u64()
            .ok_or_else(|| format!("参数 {key} 必须是正整数，收到 {number}")),
        Value::String(text) => text
            .trim()
            .parse::<u64>()
            .map_err(|_| format!("参数 {key} 不是合法的文件 ID：{text:?}")),
        other => Err(format!("参数 {key} 必须是文件 ID（数字或字符串），收到 {other}")),
    }
}

pub fn ids_argument(value: &Value, key: &str) -> Result<Vec<u64>, String> {
    let array = value
        .as_array()
        .ok_or_else(|| format!("参数 {key} 必须是文件 ID 数组"))?;
    if array.is_empty() {
        return Err(format!("参数 {key} 至少需要一个文件 ID"));
    }
    array
        .iter()
        .map(|item| id_argument(item, key))
        .collect::<Result<Vec<_>, _>>()
}
```

`tool_specs()` 返回全部 18 个工具。为控制篇幅，用一个小助手构造重复结构：

```rust
fn spec(
    name: &'static str,
    description: &'static str,
    properties: Value,
    required: &[&str],
) -> ToolSpec {
    ToolSpec {
        name,
        description,
        input_schema: json!({
            "type": "object",
            "properties": properties,
            "required": required
        }),
    }
}

fn id_property(description: &str) -> Value {
    json!({
        "type": ["integer", "string"],
        "description": format!("{description}（数字或字符串形式均可）")
    })
}

pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        spec(
            "list_files",
            "列出指定目录下的文件与子目录。parent_id 省略时使用根目录 0。",
            json!({
                "parent_id": id_property("目录的 file_id，根目录为 0"),
                "limit": { "type": "integer", "description": "返回条数上限，默认 100" }
            }),
            &[],
        ),
        spec(
            "dir_tree",
            "递归列出目录树。必须显式给出 depth，没有隐含的当前目录。",
            json!({
                "parent_id": id_property("起始目录的 file_id，根目录为 0"),
                "depth": { "type": "integer", "description": "递归深度，必须显式指定" }
            }),
            &["depth"],
        ),
        spec(
            "find_files",
            "在指定目录下按名称搜索文件与目录。",
            json!({
                "query": { "type": "string", "description": "搜索关键词" },
                "parent_id": id_property("搜索起点目录，根目录为 0"),
                "exact": { "type": "boolean", "description": "是否精确匹配名称，默认 false" },
                "dir_only": { "type": "boolean", "description": "只返回目录" },
                "file_only": { "type": "boolean", "description": "只返回文件" }
            }),
            &["query"],
        ),
        spec(
            "get_file_info",
            "按 file_id 批量查询文件详情。",
            json!({ "file_ids": {
                "type": "array",
                "description": "文件 ID 数组，元素可为数字或字符串",
                "items": { "type": ["integer", "string"] }
            }}),
            &["file_ids"],
        ),
        spec(
            "resolve_path",
            "把形如 /备份/2026 的路径解析为 file_id。无状态，每次操作前都需要解析。",
            json!({
                "path": { "type": "string", "description": "以 / 开头的绝对路径，/ 表示根目录" }
            }),
            &["path"],
        ),
        spec(
            "get_path",
            "由 file_id 反查完整路径，用于向用户复述文件位置。",
            json!({ "file_id": id_property("文件或目录的 ID") }),
            &["file_id"],
        ),
        spec(
            "user_info",
            "查看当前登录账号的信息。",
            json!({}),
            &[],
        ),
        spec(
            "check_token",
            "检查登录状态是否有效。返回 Missing/Invalid 时需要用户去终端执行 pan123 login。",
            json!({}),
            &[],
        ),
        spec(
            "create_folder",
            "在指定目录下新建文件夹。",
            json!({
                "folder_name": { "type": "string", "description": "新文件夹名称" },
                "parent_id": id_property("父目录的 file_id，根目录为 0")
            }),
            &["folder_name"],
        ),
        spec(
            "rename_file",
            "重命名文件或目录。这是修改操作。",
            json!({
                "file_id": id_property("要重命名的对象 ID"),
                "new_name": { "type": "string", "description": "新名称" }
            }),
            &["file_id", "new_name"],
        ),
        spec(
            "move_files",
            "批量移动文件或目录。这是破坏性操作，执行前必须先向用户确认。",
            json!({
                "file_ids": {
                    "type": "array",
                    "description": "要移动的对象 ID 数组",
                    "items": { "type": ["integer", "string"] }
                },
                "target_parent_id": id_property("目标目录的 file_id，根目录为 0")
            }),
            &["file_ids", "target_parent_id"],
        ),
        spec(
            "copy_files",
            "批量复制文件或目录到目标目录。",
            json!({
                "file_ids": {
                    "type": "array",
                    "description": "要复制的对象 ID 数组",
                    "items": { "type": ["integer", "string"] }
                },
                "target_parent_id": id_property("目标目录的 file_id，根目录为 0")
            }),
            &["file_ids", "target_parent_id"],
        ),
        spec(
            "delete_files",
            "批量删除文件或目录。这是不可逆操作，执行前必须先列出将被删除的对象并取得用户确认。",
            json!({
                "file_ids": {
                    "type": "array",
                    "description": "要删除的对象 ID 数组",
                    "items": { "type": ["integer", "string"] }
                }
            }),
            &["file_ids"],
        ),
        spec(
            "get_download_url",
            "获取文件的下载直链，不落盘。",
            json!({ "file_ids": {
                "type": "array",
                "description": "文件 ID 数组",
                "items": { "type": ["integer", "string"] }
            }}),
            &["file_ids"],
        ),
        spec(
            "download_files",
            "把云端文件下载到本地目录。耗时操作，会持续上报进度。dest_dir 必须是绝对路径。",
            json!({
                "file_ids": {
                    "type": "array",
                    "description": "文件 ID 数组",
                    "items": { "type": ["integer", "string"] }
                },
                "dest_dir": {
                    "type": "string",
                    "description": "本地目标目录，必须是绝对路径（相对于 server 进程工作目录解析）"
                },
                "retries": { "type": "integer", "description": "重试次数，默认 3" }
            }),
            &["file_ids", "dest_dir"],
        ),
        spec(
            "upload_file",
            "上传单个本地文件到云端目录。耗时操作，会持续上报进度。local_path 必须是绝对路径。",
            json!({
                "local_path": {
                    "type": "string",
                    "description": "本地文件路径，必须是绝对路径（相对于 server 进程工作目录解析）"
                },
                "parent_id": id_property("目标目录的 file_id，根目录为 0"),
                "duplicate": {
                    "type": "string",
                    "enum": ["keep_both", "overwrite", "cancel"],
                    "description": "遇到同名文件时的策略，默认 keep_both"
                }
            }),
            &["local_path"],
        ),
        spec(
            "upload_directory",
            "递归上传本地目录到云端。耗时操作，会持续上报进度。local_path 必须是绝对路径。",
            json!({
                "local_path": {
                    "type": "string",
                    "description": "本地目录路径，必须是绝对路径（相对于 server 进程工作目录解析）"
                },
                "parent_id": id_property("目标目录的 file_id，根目录为 0"),
                "duplicate": {
                    "type": "string",
                    "enum": ["keep_both", "overwrite", "cancel"],
                    "description": "遇到同名文件时的策略，默认 keep_both"
                },
                "retries": { "type": "integer", "description": "重试次数，默认 3" }
            }),
            &["local_path"],
        ),
        spec(
            "list_upload_sessions",
            "列出本地记录的未完成上传会话（断点续传依据）。",
            json!({ "clear": { "type": "boolean", "description": "是否清空全部会话记录" } }),
            &[],
        ),
    ]
}
```

已核对 `crates/pan123-sdk/src/models.rs:105`：`FileInfo::is_dir()` 判定为 `file_type != 0`，上文的 `format_file_list` 与后续工具据此标注 `[目录]` / `[文件]`。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p pan123-cli mcp::tools -- --nocapture`
Expected: 11 passed。

覆盖 Review Focus 第 3 条：`id_argument_reports_the_offending_key`、`id_argument_rejects_value_beyond_u64`。

- [ ] **Step 5: 提交**

```bash
git add crates/pan123-cli/src/mcp/tools.rs crates/pan123-cli/src/mcp/mod.rs
git commit -m "feat(cli): 定义 18 个 MCP 工具 schema 与 id 参数解析"
```

---

### Task 8: 只读工具与路径解析实现

**Files:**
- Modify: `crates/pan123-cli/src/mcp/tools.rs`
- Test: 同文件内联测试

**Interfaces:**
- Consumes: Task 7 的 `tool_specs` / `id_argument` / `ids_argument`；Task 5 的 `ToolHandler`、`ToolCallOutcome`
- Produces:
  - `impl ToolHandler for Pan123Tools`（本任务先实现只读分支，写操作在 Task 9 补）
  - `pub fn format_file_list(items: &[FileInfo]) -> String`
  - `pub fn parse_path_segments(path: &str) -> Result<Vec<String>, String>`

- [ ] **Step 1: 写失败测试**

在 `crates/pan123-cli/src/mcp/tools.rs` 的 `tests` 模块内追加：

```rust
    use pan123_sdk::FileInfo;
    use serde_json::json as j;

    fn file(id: u64, parent: u64, name: &str, file_type: u8, size: u64) -> FileInfo {
        serde_json::from_value(j!({
            "FileId": id,
            "ParentFileId": parent,
            "FileName": name,
            "Type": file_type,
            "Size": size
        }))
        .expect("fixture should deserialize")
    }

    #[test]
    fn parse_path_segments_splits_absolute_path() {
        assert_eq!(parse_path_segments("/备份/2026").unwrap(), vec!["备份", "2026"]);
    }

    #[test]
    fn parse_path_segments_root_is_empty() {
        assert_eq!(parse_path_segments("/").unwrap(), Vec::<String>::new());
        assert_eq!(parse_path_segments("").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn parse_path_segments_rejects_relative_path() {
        let err = parse_path_segments("备份/2026").unwrap_err();
        assert!(err.contains("绝对路径"), "错误信息：{err}");
    }

    #[test]
    fn parse_path_segments_rejects_dot_segments() {
        assert!(parse_path_segments("/a/../b").is_err());
        assert!(parse_path_segments("/a/.").is_err());
    }

    #[test]
    fn format_file_list_marks_directories_and_sizes() {
        let items = vec![
            file(1, 0, "照片", 1, 0),
            file(2, 0, "报告.pdf", 0, 2048),
        ];
        let text = format_file_list(&items);
        assert!(text.contains("照片"), "缺少目录名：{text}");
        assert!(text.contains("[目录]"), "缺少目录标记：{text}");
        assert!(text.contains("报告.pdf"), "缺少文件名：{text}");
        assert!(text.contains("2048"), "缺少大小：{text}");
    }

    #[test]
    fn format_file_list_reports_empty_directory() {
        assert!(format_file_list(&[]).contains("空"));
    }

    #[test]
    fn unknown_tool_returns_is_error() {
        let client = Pan123Client::new(Some("dummy-token".into())).expect("client builds offline");
        let tools = Pan123Tools { client };
        let ctx = crate::mcp::protocol::CallContext {
            progress: crate::mcp::protocol::ProgressSink::discarding(),
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        let outcome = tools.call("no_such_tool", &j!({}), &ctx);
        assert!(outcome.is_error);
        assert!(outcome.text.contains("no_such_tool"));
    }

    #[test]
    fn resolve_path_on_root_needs_no_api_call() {
        let client = Pan123Client::new(Some("dummy-token".into())).expect("client builds offline");
        let tools = Pan123Tools { client };
        let ctx = crate::mcp::protocol::CallContext {
            progress: crate::mcp::protocol::ProgressSink::discarding(),
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        let outcome = tools.call("resolve_path", &j!({ "path": "/" }), &ctx);
        assert!(!outcome.is_error, "根目录解析不应依赖网络：{}", outcome.text);
        assert!(outcome.text.contains('0'));
    }

    #[test]
    fn invalid_id_argument_returns_is_error_naming_the_key() {
        let client = Pan123Client::new(Some("dummy-token".into())).expect("client builds offline");
        let tools = Pan123Tools { client };
        let ctx = crate::mcp::protocol::CallContext {
            progress: crate::mcp::protocol::ProgressSink::discarding(),
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        let outcome = tools.call("get_file_info", &j!({ "file_ids": ["abc"] }), &ctx);
        assert!(outcome.is_error);
        assert!(outcome.text.contains("file_ids"), "实际：{}", outcome.text);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p pan123-cli mcp::tools::tests -- --nocapture`
Expected: 编译失败，`cannot find function 'parse_path_segments'`。

- [ ] **Step 3: 实现**

在 `crates/pan123-cli/src/mcp/tools.rs` 顶部补充：

```rust
use std::sync::atomic::Ordering;

use pan123_sdk::{FileInfo, Pan123Error};

use crate::mcp::protocol::{CallContext, ToolCallOutcome, ToolHandler};
```

加入纯函数：

```rust
/// 把绝对路径拆成目录段。`/` 或空串返回空向量。拒绝相对路径与 `.` / `..` 段。
pub fn parse_path_segments(path: &str) -> Result<Vec<String>, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return Ok(Vec::new());
    }
    if !trimmed.starts_with('/') {
        return Err(format!("路径必须是绝对路径（以 / 开头），收到 {path:?}"));
    }
    let mut segments = Vec::new();
    for segment in trimmed.split('/') {
        if segment.is_empty() {
            continue;
        }
        if segment == "." || segment == ".." {
            return Err(format!("路径中不允许出现 . 或 .. 段：{path:?}"));
        }
        segments.push(segment.to_string());
    }
    Ok(segments)
}

pub fn format_file_list(items: &[FileInfo]) -> String {
    if items.is_empty() {
        return "该目录为空".to_string();
    }
    items
        .iter()
        .map(|item| {
            if item.is_dir() {
                format!("[目录] {} (id={})", item.file_name, item.file_id)
            } else {
                format!(
                    "[文件] {} (id={}, {} 字节)",
                    item.file_name, item.file_id, item.size
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 把 Pan123Error 翻译成给模型看的、可操作的说明。
pub fn explain_error(err: &Pan123Error) -> String {
    match err {
        Pan123Error::AuthRequired | Pan123Error::InvalidToken => {
            "登录状态无效。请让用户在终端执行 `pan123 login` 扫码登录后重试。".to_string()
        }
        Pan123Error::NotFound(what) => format!("未找到：{what}。请确认 file_id 是否正确。"),
        Pan123Error::Cancelled => "操作已被取消。".to_string(),
        Pan123Error::InvalidPath(path) => format!("路径不合法：{path}。"),
        other => format!("操作失败：{other}"),
    }
}

/// 把闭包里的 SDK 调用收敛成统一的 outcome，顺带做取消检查。
fn run_read_only<F>(ctx: &CallContext, operation: F) -> ToolCallOutcome
where
    F: FnOnce() -> Result<String, Pan123Error>,
{
    if ctx.cancel.load(Ordering::SeqCst) {
        return ToolCallOutcome { text: "操作已被取消。".into(), is_error: true };
    }
    match operation() {
        Ok(text) => ToolCallOutcome { text, is_error: false },
        Err(err) => ToolCallOutcome { text: explain_error(&err), is_error: true },
    }
}
```

加入 `ToolHandler` 实现（本任务只写只读分支，`unimplemented` 分支留给 Task 9，需保证默认分支返回 isError 而非 panic）：

```rust
impl ToolHandler for Pan123Tools {
    fn list_tools(&self) -> Vec<ToolSpec> {
        tool_specs()
    }

    fn call(&self, name: &str, args: &Value, ctx: &CallContext) -> ToolCallOutcome {
        match name {
            "list_files" => run_read_only(ctx, || {
                let parent_id = match args.get("parent_id") {
                    Some(value) => {
                        id_argument(value, "parent_id").map_err(Pan123Error::Operation)?
                    }
                    None => 0,
                };
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(100) as u32;
                let items = self.client.get_file_list(parent_id, 1, limit)?;
                Ok(format!("目录 {} 下共 {} 项：\n{}", parent_id, items.len(), format_file_list(&items)))
            }),
            "get_file_info" => run_read_only(ctx, || {
                let raw = args.get("file_ids").ok_or_else(|| {
                    Pan123Error::Operation("缺少参数 file_ids".into())
                })?;
                let ids = ids_argument(raw, "file_ids").map_err(Pan123Error::Operation)?;
                let items = self.client.get_file_info(&ids)?;
                Ok(format_file_list(&items))
            }),
            "get_path" => run_read_only(ctx, || {
                let raw = args.get("file_id").ok_or_else(|| {
                    Pan123Error::Operation("缺少参数 file_id".into())
                })?;
                let id = id_argument(raw, "file_id").map_err(Pan123Error::Operation)?;
                let chain = self.client.get_path_chain(id)?;
                let path = chain
                    .iter()
                    .map(|item| item.file_name.as_str())
                    .collect::<Vec<_>>()
                    .join("/");
                Ok(format!("/{} (id={})", path, id))
            }),
            "user_info" => run_read_only(ctx, || {
                Ok(serde_json::to_string_pretty(&self.client.get_user_info()?)?)
            }),
            "check_token" => {
                use pan123_sdk::TokenCheckStatus;
                let text = match self.client.check_token_status() {
                    TokenCheckStatus::Valid => "登录状态有效。".to_string(),
                    TokenCheckStatus::Missing => {
                        "尚未登录。请让用户在终端执行 `pan123 login` 扫码登录。".to_string()
                    }
                    TokenCheckStatus::Invalid => {
                        "登录已失效。请让用户在终端执行 `pan123 login` 重新扫码。".to_string()
                    }
                    TokenCheckStatus::Unreachable(message) => {
                        format!("无法验证登录状态（网络问题）：{message}")
                    }
                };
                ToolCallOutcome { text, is_error: false }
            }
            "resolve_path" => run_read_only(ctx, || {
                let raw = args.get("path").ok_or_else(|| {
                    Pan123Error::Operation("缺少参数 path".into())
                })?;
                let path = raw.as_str().ok_or_else(|| {
                    Pan123Error::Operation("参数 path 必须是字符串".into())
                })?;
                let segments = parse_path_segments(path).map_err(Pan123Error::Operation)?;
                let mut current = 0u64;
                for segment in &segments {
                    let folder = self.client.find_child_folder_by_name(current, segment)?;
                    match folder {
                        Some(item) => current = item.file_id,
                        None => {
                            return Err(Pan123Error::NotFound(format!(
                                "路径 {path} 中的目录 {segment}"
                            )));
                        }
                    }
                }
                Ok(format!("{} -> file_id={}", path, current))
            }),
            other => ToolCallOutcome {
                text: format!("未知工具：{other}"),
                is_error: true,
            },
        }
    }
}
```

已核对签名（`client.rs:1422` 与 `client.rs:1433`），与上文用法一致：

- `find_child_folder_by_name(&self, parent_id: u64, folder_name: &str) -> Result<Option<FileInfo>>`
- `get_path_chain(&self, file_id: u64) -> Result<Vec<FileInfo>>`；`file_id == 0` 时返回空向量

`dir_tree` 与 `find_files` 依赖 CLI 内已有的树遍历与匹配逻辑，本任务不实现——它们的分支落入 `other =>` 返回 isError，Task 9 补上。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p pan123-cli mcp::tools::tests -- --nocapture`
Expected: 全部通过。

- [ ] **Step 5: 提交**

```bash
git add crates/pan123-cli/src/mcp/tools.rs
git commit -m "feat(cli): MCP 只读工具与路径解析"
```

---

### Task 9: 写操作与传输工具实现

**Files:**
- Modify: `crates/pan123-cli/src/mcp/tools.rs`
- Test: 同文件内联测试

**Interfaces:**
- Consumes: Task 8 的 `run_read_only` / `parse_path_segments`；Task 6 的 `ProgressSink`
- Produces:
  - `dir_tree`、`find_files`、`create_folder`、`rename_file`、`move_files`、`copy_files`、`delete_files`、`get_download_url`、`download_files`、`upload_file`、`upload_directory`、`list_upload_sessions` 的实现
  - `pub fn absolute_local_path(raw: &str, key: &str) -> Result<PathBuf, String>`
  - `pub fn transfer_callback(ctx: &CallContext) -> pan123_sdk::ProgressCallback`

- [ ] **Step 1: 写失败测试**

在 `crates/pan123-cli/src/mcp/tools.rs` 的 `tests` 模块内追加：

```rust
    #[test]
    fn absolute_local_path_accepts_absolute() {
        let path = absolute_local_path("/tmp/x", "local_path").unwrap();
        assert!(path.is_absolute());
    }

    #[test]
    fn absolute_local_path_rejects_relative() {
        let err = absolute_local_path("relative/x", "local_path").unwrap_err();
        assert!(err.contains("local_path"), "实际：{err}");
        assert!(err.contains("绝对路径"), "实际：{err}");
    }

    #[test]
    fn absolute_local_path_rejects_parent_traversal() {
        let err = absolute_local_path("/tmp/../../etc/passwd", "dest_dir").unwrap_err();
        assert!(err.contains("dest_dir"), "实际：{err}");
    }

    #[test]
    fn duplicate_parsing_defaults_to_keep_both() {
        assert_eq!(duplicate_argument(&j!({})).unwrap(), DuplicateMode::KeepBoth);
        assert_eq!(
            duplicate_argument(&j!({ "duplicate": "overwrite" })).unwrap(),
            DuplicateMode::Overwrite
        );
        assert_eq!(
            duplicate_argument(&j!({ "duplicate": "cancel" })).unwrap(),
            DuplicateMode::Cancel
        );
        assert!(duplicate_argument(&j!({ "duplicate": "nonsense" })).is_err());
    }

    #[test]
    fn write_tools_are_dispatched_not_reported_unknown() {
        // 未登录时写操作应返回鉴权相关的 isError，而不是“未知工具”。
        let client = Pan123Client::new(Some("dummy-token".into())).expect("client builds offline");
        let tools = Pan123Tools { client };
        let ctx = crate::mcp::protocol::CallContext {
            progress: crate::mcp::protocol::ProgressSink::discarding(),
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        for (name, args) in [
            ("create_folder", j!({ "folder_name": "x" })),
            ("rename_file", j!({ "file_id": 1, "new_name": "y" })),
            ("delete_files", j!({ "file_ids": [1] })),
            ("move_files", j!({ "file_ids": [1], "target_parent_id": 0 })),
            ("copy_files", j!({ "file_ids": [1], "target_parent_id": 0 })),
            ("get_download_url", j!({ "file_ids": [1] })),
            ("list_upload_sessions", j!({})),
            ("find_files", j!({ "query": "x" })),
            ("dir_tree", j!({ "depth": 1 })),
        ] {
            let outcome = tools.call(name, &args, &ctx);
            assert!(
                !outcome.text.contains("未知工具"),
                "{name} 未接线，实际返回：{}",
                outcome.text
            );
        }
    }

    #[test]
    fn upload_rejects_relative_path_before_touching_network() {
        let client = Pan123Client::new(Some("dummy-token".into())).expect("client builds offline");
        let tools = Pan123Tools { client };
        let ctx = crate::mcp::protocol::CallContext {
            progress: crate::mcp::protocol::ProgressSink::discarding(),
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        let outcome = tools.call("upload_file", &j!({ "local_path": "rel.txt" }), &ctx);
        assert!(outcome.is_error);
        assert!(outcome.text.contains("local_path"), "实际：{}", outcome.text);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p pan123-cli mcp::tools::tests -- --nocapture`
Expected: 失败——`write_tools_are_dispatched_not_reported_unknown` 报多个工具返回“未知工具”；`absolute_local_path` 等函数不存在导致编译失败。

- [ ] **Step 3: 实现**

在 `crates/pan123-cli/src/mcp/tools.rs` 顶部补充：

```rust
use std::path::{Component, Path, PathBuf};

use pan123_sdk::{
    DownloadOptions, DuplicateMode, ProgressCallback, RetryPolicy, TransferEvent,
    TransferOptions, UploadOptions,
};
```

加入辅助函数：

```rust
/// 校验本地路径是绝对路径且不含 `..`。避免文件落到 server 进程工作目录下的意外位置。
pub fn absolute_local_path(raw: &str, key: &str) -> Result<PathBuf, String> {
    let path = Path::new(raw);
    if !path.is_absolute() {
        return Err(format!(
            "参数 {key} 必须是绝对路径（相对于 server 进程工作目录解析），收到 {raw:?}"
        ));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!("参数 {key} 不允许包含 .. 段：{raw:?}"));
    }
    Ok(path.to_path_buf())
}

pub fn duplicate_argument(args: &Value) -> Result<DuplicateMode, String> {
    match args.get("duplicate").and_then(Value::as_str) {
        None => Ok(DuplicateMode::KeepBoth),
        Some("keep_both") => Ok(DuplicateMode::KeepBoth),
        Some("overwrite") => Ok(DuplicateMode::Overwrite),
        Some("cancel") => Ok(DuplicateMode::Cancel),
        Some(other) => Err(format!(
            "参数 duplicate 只能是 keep_both / overwrite / cancel，收到 {other:?}"
        )),
    }
}

/// 把 SDK 的 TransferEvent 桥接到 MCP 进度通知。没有 progressToken 时 report 自身会丢弃。
pub fn transfer_callback(ctx: &CallContext) -> ProgressCallback {
    let sink = ctx.progress.clone();
    std::sync::Arc::new(move |event: TransferEvent| match event {
        TransferEvent::Progress {
            bytes, total_bytes, ..
        } => sink.report(bytes, total_bytes, None),
        TransferEvent::Started { total_bytes, .. } => {
            sink.report(0, total_bytes, Some("开始传输"));
        }
        TransferEvent::Retrying { attempt, .. } => {
            sink.report(0, None, Some(&format!("第 {attempt} 次重试")))
        }
        TransferEvent::Finished { total_bytes, .. } => {
            sink.report(total_bytes, Some(total_bytes), Some("传输完成"));
        }
        TransferEvent::Failed { message, .. } => {
            sink.report(0, None, Some(&format!("传输失败：{message}")))
        }
    })
}

fn retry_policy_from(args: &Value) -> RetryPolicy {
    let mut policy = RetryPolicy::default();
    if let Some(retries) = args.get("retries").and_then(Value::as_u64) {
        policy.max_attempts = retries.max(1) as usize;
    }
    policy
}

fn owned_transfer_options(ctx: &CallContext, retry: RetryPolicy) -> TransferOptions {
    TransferOptions {
        retry,
        cancel: Some(ctx.cancel.clone()),
        ..Default::default()
    }
}
```

在 `call` 的 `match name` 中，把 Task 8 的 `other =>` 分支之前插入以下分支：

```rust
            "create_folder" => run_read_only(ctx, || {
                let name = args
                    .get("folder_name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| Pan123Error::Operation("缺少参数 folder_name".into()))?;
                let parent_id = match args.get("parent_id") {
                    Some(value) => id_argument(value, "parent_id")
                        .map_err(Pan123Error::Operation)?,
                    None => 0,
                };
                let created = self.client.create_folder(name, parent_id)?;
                Ok(format!("已创建 [目录] {} (id={})", created.file_name, created.file_id))
            }),
            "rename_file" => run_read_only(ctx, || {
                let raw = args.get("file_id").ok_or_else(|| {
                    Pan123Error::Operation("缺少参数 file_id".into())
                })?;
                let id = id_argument(raw, "file_id").map_err(Pan123Error::Operation)?;
                let new_name = args
                    .get("new_name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| Pan123Error::Operation("缺少参数 new_name".into()))?;
                self.client.rename_file(id, new_name)?;
                Ok(format!("已把 id={id} 重命名为 {new_name}"))
            }),
            "move_files" => run_read_only(ctx, || {
                let ids = ids_argument(
                    args.get("file_ids")
                        .ok_or_else(|| Pan123Error::Operation("缺少参数 file_ids".into()))?,
                    "file_ids",
                )
                .map_err(Pan123Error::Operation)?;
                let target = id_argument(
                    args.get("target_parent_id").ok_or_else(|| {
                        Pan123Error::Operation("缺少参数 target_parent_id".into())
                    })?,
                    "target_parent_id",
                )
                .map_err(Pan123Error::Operation)?;
                self.client.move_files(&ids, target)?;
                Ok(format!("已移动 {} 项到 id={target}", ids.len()))
            }),
            "copy_files" => run_read_only(ctx, || {
                let ids = ids_argument(
                    args.get("file_ids")
                        .ok_or_else(|| Pan123Error::Operation("缺少参数 file_ids".into()))?,
                    "file_ids",
                )
                .map_err(Pan123Error::Operation)?;
                let target = id_argument(
                    args.get("target_parent_id").ok_or_else(|| {
                        Pan123Error::Operation("缺少参数 target_parent_id".into())
                    })?,
                    "target_parent_id",
                )
                .map_err(Pan123Error::Operation)?;
                self.client.copy_files(&ids, target)?;
                Ok(format!("已复制 {} 项到 id={target}", ids.len()))
            }),
            "delete_files" => run_read_only(ctx, || {
                let ids = ids_argument(
                    args.get("file_ids")
                        .ok_or_else(|| Pan123Error::Operation("缺少参数 file_ids".into()))?,
                    "file_ids",
                )
                .map_err(Pan123Error::Operation)?;
                self.client.delete_files(&ids)?;
                Ok(format!("已删除 {} 项", ids.len()))
            }),
            "get_download_url" => run_read_only(ctx, || {
                let ids = ids_argument(
                    args.get("file_ids")
                        .ok_or_else(|| Pan123Error::Operation("缺少参数 file_ids".into()))?,
                    "file_ids",
                )
                .map_err(Pan123Error::Operation)?;
                let link = self.client.get_download_link(&ids)?;
                Ok(format!("文件名：{}\n下载直链：{}", link.filename, link.url))
            }),
            "download_files" => {
                let prepared = (|| -> Result<(Vec<u64>, PathBuf), Pan123Error> {
                    let ids = ids_argument(
                        args.get("file_ids")
                            .ok_or_else(|| Pan123Error::Operation("缺少参数 file_ids".into()))?,
                        "file_ids",
                    )
                    .map_err(Pan123Error::Operation)?;
                    let raw = args.get("dest_dir").and_then(Value::as_str).ok_or_else(|| {
                        Pan123Error::Operation("缺少参数 dest_dir（必须为绝对路径）".into())
                    })?;
                    let dir = absolute_local_path(raw, "dest_dir")
                        .map_err(Pan123Error::Operation)?;
                    Ok((ids, dir))
                })();
                match prepared {
                    Err(err) => ToolCallOutcome {
                        text: explain_error(&err),
                        is_error: true,
                    },
                    Ok((ids, dir)) => {
                        let options = DownloadOptions {
                            transfer: owned_transfer_options(ctx, retry_policy_from(args)),
                            resume: true,
                        };
                        let callback = transfer_callback(ctx);
                        match self.client.download_files_with(&ids, &dir, options, Some(callback)) {
                            Ok(file) => ToolCallOutcome {
                                text: format!(
                                    "已下载到 {}（{} 字节）",
                                    file.file_path.display(),
                                    file.size
                                ),
                                is_error: false,
                            },
                            Err(err) => ToolCallOutcome {
                                text: explain_error(&err),
                                is_error: true,
                            },
                        }
                    }
                }
            }
            "upload_file" | "upload_directory" => {
                let is_directory = name == "upload_directory";
                let prepared = (|| -> Result<(PathBuf, u64, DuplicateMode), Pan123Error> {
                    let raw = args.get("local_path").and_then(Value::as_str).ok_or_else(|| {
                        Pan123Error::Operation("缺少参数 local_path（必须为绝对路径）".into())
                    })?;
                    let path =
                        absolute_local_path(raw, "local_path").map_err(Pan123Error::Operation)?;
                    let parent_id = match args.get("parent_id") {
                        Some(value) => id_argument(value, "parent_id")
                            .map_err(Pan123Error::Operation)?,
                        None => 0,
                    };
                    let duplicate =
                        duplicate_argument(args).map_err(Pan123Error::Operation)?;
                    Ok((path, parent_id, duplicate))
                })();
                match prepared {
                    Err(err) => ToolCallOutcome {
                        text: explain_error(&err),
                        is_error: true,
                    },
                    Ok((path, parent_id, duplicate)) => {
                        if is_directory && !path.is_dir() {
                            return ToolCallOutcome {
                                text: format!("local_path 不是目录：{}", path.display()),
                                is_error: true,
                            };
                        }
                        if !is_directory && !path.is_file() {
                            return ToolCallOutcome {
                                text: format!("local_path 不是文件：{}", path.display()),
                                is_error: true,
                            };
                        }
                        let options = UploadOptions {
                            transfer: owned_transfer_options(ctx, retry_policy_from(args)),
                            resume: true,
                        };
                        let callback = transfer_callback(ctx);
                        if is_directory {
                            match self.client.upload_directory_with(
                                &path,
                                parent_id,
                                duplicate,
                                options,
                                Some(callback),
                            ) {
                                Ok(report) => ToolCallOutcome {
                                    text: format!(
                                        "上传完成：成功 {} 个，失败 {} 个",
                                        report.uploaded_count(),
                                        report.failed_count()
                                    ),
                                    is_error: !report.is_complete_success(),
                                },
                                Err(err) => ToolCallOutcome {
                                    text: explain_error(&err),
                                    is_error: true,
                                },
                            }
                        } else {
                            match self.client.upload_file_with(
                                &path,
                                parent_id,
                                duplicate,
                                options,
                                Some(callback),
                            ) {
                                Ok(info) => ToolCallOutcome {
                                    text: format!(
                                        "已上传 {} (id={})",
                                        info.file_name, info.file_id
                                    ),
                                    is_error: false,
                                },
                                Err(err) => ToolCallOutcome {
                                    text: explain_error(&err),
                                    is_error: true,
                                },
                            }
                        }
                    }
                }
            }
            "list_upload_sessions" => run_read_only(ctx, || {
                let clear = args.get("clear").and_then(Value::as_bool).unwrap_or(false);
                if clear {
                    let removed = pan123_sdk::upload_session::clear()?;
                    return Ok(format!("已清空 {removed} 条上传会话记录。"));
                }
                let sessions = pan123_sdk::upload_session::list();
                if sessions.is_empty() {
                    return Ok("没有未完成的上传会话记录。".to_string());
                }
                Ok(format!("共 {} 条未完成上传会话", sessions.len()))
            }),
            "dir_tree" => run_read_only(ctx, || {
                let depth = args
                    .get("depth")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| Pan123Error::Operation("必须显式提供参数 depth".into()))?;
                let parent_id = match args.get("parent_id") {
                    Some(value) => id_argument(value, "parent_id")
                        .map_err(Pan123Error::Operation)?,
                    None => 0,
                };
                let mut out = String::new();
                self.render_tree(parent_id, 0, depth as usize, &mut out)?;
                Ok(out)
            }),
            "find_files" => run_read_only(ctx, || {
                let query = args
                    .get("query")
                    .and_then(Value::as_str)
                    .ok_or_else(|| Pan123Error::Operation("缺少参数 query".into()))?;
                let parent_id = match args.get("parent_id") {
                    Some(value) => id_argument(value, "parent_id")
                        .map_err(Pan123Error::Operation)?,
                    None => 0,
                };
                let exact = args.get("exact").and_then(Value::as_bool).unwrap_or(false);
                let dir_only = args.get("dir_only").and_then(Value::as_bool).unwrap_or(false);
                let file_only = args
                    .get("file_only")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let mut matches = Vec::new();
                self.collect_matches(parent_id, query, exact, dir_only, file_only, &mut matches)?;
                if matches.is_empty() {
                    return Ok(format!("没有匹配 {query:?} 的文件"));
                }
                Ok(format_file_list(&matches))
            }),
```

并在 `impl Pan123Tools` 中加入两个递归辅助方法：

```rust
impl Pan123Tools {
    fn render_tree(
        &self,
        parent_id: u64,
        level: usize,
        max_depth: usize,
        out: &mut String,
    ) -> Result<(), Pan123Error> {
        if level >= max_depth {
            return Ok(());
        }
        let items = self.client.get_file_list(parent_id, 1, 100)?;
        for item in items {
            out.push_str(&"  ".repeat(level));
            if item.is_dir() {
                out.push_str(&format!("[目录] {}\n", item.file_name));
                self.render_tree(item.file_id, level + 1, max_depth, out)?;
            } else {
                out.push_str(&format!("[文件] {} ({} 字节)\n", item.file_name, item.size));
            }
        }
        Ok(())
    }

    fn collect_matches(
        &self,
        parent_id: u64,
        query: &str,
        exact: bool,
        dir_only: bool,
        file_only: bool,
        out: &mut Vec<FileInfo>,
    ) -> Result<(), Pan123Error> {
        let items = self.client.get_file_list(parent_id, 1, 100)?;
        for item in items {
            let name_matches = if exact {
                item.file_name == query
            } else {
                item.file_name.contains(query)
            };
            if name_matches {
                let kind_ok = if dir_only {
                    item.is_dir()
                } else if file_only {
                    !item.is_dir()
                } else {
                    true
                };
                if kind_ok {
                    out.push(item.clone());
                }
            }
            if item.is_dir() {
                self.collect_matches(item.file_id, query, exact, dir_only, file_only, out)?;
            }
        }
        Ok(())
    }
}
```

所用 `pan123_sdk::upload_session` 公开函数已核对为 `list() -> Vec<UploadSession>`（非 `Result`）与 `clear() -> Result<usize>`，二者都只读本地文件、不触网：

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p pan123-cli mcp::tools -- --nocapture`
Expected: 全部通过。

覆盖 Review Focus 第 4 条：`absolute_local_path_rejects_relative`、`absolute_local_path_rejects_parent_traversal`、`upload_rejects_relative_path_before_touching_network`。

- [ ] **Step 5: 提交**

```bash
git add crates/pan123-cli/src/mcp/tools.rs
git commit -m "feat(cli): MCP 写操作与传输工具实现"
```

---

## Phase 4：CLI 接线与输出

### Task 10: `pan123 mcp` 子命令

**Files:**
- Modify: `crates/pan123-cli/src/cli.rs`（`Command` 枚举与 `execute`）
- Test: 手工冒烟（无自动化测试，见 Step 4）

**Interfaces:**
- Consumes: Task 6 的 `run_server`、Task 9 的 `Pan123Tools`
- Produces: `pan123 mcp` 子命令

- [ ] **Step 1: 加入子命令**

在 `crates/pan123-cli/src/cli.rs` 的 `enum Command` 中，`Shell` 之后加入：

```rust
    /// 以 MCP（Model Context Protocol）服务器的形式运行，通过 stdio 与客户端通信。
    Mcp,
```

在 `execute` 的 `match command` 中加入分支（放在 `Command::Clear` 之前）：

```rust
            Command::Mcp => {
                // MCP 路径下 stdout 属于协议流，绝不能有任何日志写进去。
                let client = Pan123Client::with_rate_limiter(config::load_token(), None)?;
                let tools = std::sync::Arc::new(crate::mcp::tools::Pan123Tools { client });
                let stdin = std::io::stdin();
                let reader = std::io::BufReader::new(stdin.lock());
                crate::mcp::protocol::run_server(
                    tools,
                    reader,
                    std::io::stdout(),
                )
                .map_err(|err| Pan123Error::Operation(format!("MCP server 异常退出：{err}")))?;
                Ok(())
            }
```

注意：此处**不能**调用 `self.client_mut()?` 或 `ensure_auth()`——未登录时 server 也必须正常启动，登录问题留到工具调用时以 `isError` 报出。

- [ ] **Step 2: 编译并跑全部测试**

Run: `cargo build -p pan123-cli && cargo test --workspace`
Expected: 编译通过，全部测试通过。

- [ ] **Step 3: clippy 与 fmt**

Run: `cargo fmt --all && cargo clippy --all-features -- -D warnings`
Expected: 无告警、无格式差异。

- [ ] **Step 4: 手工冒烟**

**不要用 PowerShell 管道喂 stdin**：Windows PowerShell 5.1 会给管道内容加上 UTF-8 BOM，
首行因此解析失败（`-32700`），后续请求又因 `initialize` 未成功而全部收到 `-32600`。
把请求写进文件再重定向：

```powershell
@'
{ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18" } }
{ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }
{ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "check_token", "arguments": {} } }
'@ | Set-Content -Encoding ascii -NoNewline -Path smoke.jsonl
.\target\debug\pan123.exe mcp < smoke.jsonl
Remove-Item smoke.jsonl
```

（POSIX 下等价的写法是 `printf '%s\n' '<json>' '<json>' '<json>' | ./target/debug/pan123 mcp`。）

Expected：stdout 恰好三行 JSON；每行都能被 `ConvertFrom-Json` 解析；`tools/list` 返回 18 个工具；
第 3 行的 `result.content[0].text` 说明登录状态（未登录/已失效时指引去跑 `pan123 login`，且 `isError` 为 `false`）；
**stdout 中不出现任何非 JSON 内容**。可用 `Out-File` 落盘后检查字节数与转义符，确认没有 ANSI 颜色码（`0x1b`）混入。
用一个已登录的 token 重跑，第 3 行应显示"登录状态有效"。

- [ ] **Step 5: 提交**

```bash
git add crates/pan123-cli/src/cli.rs
git commit -m "feat(cli): 新增 pan123 mcp 子命令"
```

---

### Task 11: `ls` / `tree` / `find` 的 `--json`

**Files:**
- Modify: `crates/pan123-cli/src/cli.rs`（`Command::Ls` / `Command::Tree` / `Command::Find` 及其参数）
- Modify: `crates/pan123-cli/src/cli.rs:1338`（`FileInfoWithPath` 加 `Serialize`）
- Modify: `crates/pan123-cli/src/cli.rs:941,1008,1213`（三个 print 函数）
- Test: 手工冒烟

**Interfaces:**
- Consumes: 无
- Produces: `ls --json`、`tree --json`、`find --json`

**注意：`info` 已经是 JSON 输出**（`cli.rs:201` 无条件 `serde_json::to_string_pretty`），无需改动，也不要给它加 `--json`。

- [ ] **Step 1: 加参数**

给 `Command::Ls`、`Command::Tree`、`Command::Find` 三个变体各加一个字段：

```rust
        /// 以 JSON 输出，便于程序化解析。
        #[arg(long)]
        json: bool,
```

- [ ] **Step 2: 加序列化**

在 `crates/pan123-cli/src/cli.rs` 顶部 import 区加入：

```rust
use serde::Serialize;
```

把 `FileInfoWithPath` 改为：

```rust
#[derive(Serialize)]
struct FileInfoWithPath {
    path: String,
    item: FileInfo,
}
```

`FileInfo` 在 SDK 中已派生 `Serialize`（`models.rs:82` 前的 derive），无需改动。

- [ ] **Step 3: 接线**

`Command::Ls` 分支改为：

```rust
            Command::Ls { parent, limit, json } => {
                self.ensure_auth()?;
                let cwd = config::load_cwd();
                let parent_id = match parent {
                    Some(target) => self.resolve_directory_ref(&cwd, &target)?.file_id,
                    None => cwd.file_id,
                };
                let items = self.client()?.get_file_list(parent_id, 1, limit)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&items)?);
                } else {
                    self.print_ls(&items);
                }
                Ok(())
            }
```

`Command::Tree { parent, depth, json }` 分支：`json` 为真时改为收集结构而非直接打印。把 `print_tree` 改造成同时可产出数据的形态——新增：

```rust
    fn collect_tree(
        &self,
        parent_id: u64,
        prefix: String,
        level: usize,
        max_depth: usize,
        out: &mut Vec<FileInfoWithPath>,
    ) -> Result<()> {
        if level > max_depth {
            return Ok(());
        }
        let items = self.client()?.get_file_list(parent_id, 1, 100)?;
        for item in items {
            let path = format!("{}/{}", prefix.trim_end_matches('/'), item.file_name);
            let is_dir = item.is_dir();
            let file_id = item.file_id;
            out.push(FileInfoWithPath { path: path.clone(), item });
            if is_dir && level < max_depth {
                self.collect_tree(file_id, path, level + 1, max_depth, out)?;
            }
        }
        Ok(())
    }
```

分支改为：

```rust
            Command::Tree { parent, depth, json } => {
                self.ensure_auth()?;
                let cwd = config::load_cwd();
                let resolved = match parent {
                    Some(target) => self.resolve_directory_ref(&cwd, &target)?,
                    None => cwd,
                };
                if json {
                    let mut entries = Vec::new();
                    self.collect_tree(resolved.file_id, resolved.path.clone(), 1, depth, &mut entries)?;
                    println!("{}", serde_json::to_string_pretty(&entries)?);
                } else {
                    println!("{}", resolved.path.bright_blue().bold());
                    self.print_tree(resolved.file_id, String::new(), 1, depth)?;
                }
                Ok(())
            }
```

`Command::Find` 分支的结尾改为：

```rust
                if json {
                    println!("{}", serde_json::to_string_pretty(&results)?);
                } else {
                    self.print_find_results(&results);
                }
                Ok(())
```

（`Find` 的参数列表中加入 `json`，与 `Ls`/`Tree` 同样写法。）

- [ ] **Step 4: 验证**

Run: `cargo build -p pan123-cli && cargo test --workspace`
Expected: 通过。

手工冒烟（需要已登录）：

```powershell
.\target\debug\pan123.exe ls --json | ConvertFrom-Json | Select-Object -First 1
.\target\debug\pan123.exe find --json pan123 | ConvertFrom-Json | Select-Object -First 1
```

Expected：两条命令都能被 `ConvertFrom-Json` 解析，输出无 ANSI 颜色码。

- [ ] **Step 5: 提交**

```bash
git add crates/pan123-cli/src/cli.rs
git commit -m "feat(cli): ls/tree/find 支持 --json 结构化输出"
```

---

### Task 12: 非 TTY 自动去色与 `--no-color`

**Files:**
- Modify: `crates/pan123-cli/src/cli.rs`
- Test: 手工冒烟（TTY 状态无法在单测中稳定模拟，用管道验证）

**Interfaces:**
- Consumes: 无
- Produces: 全局 `--no-color`；`stdout` 非 TTY 时自动关闭颜色与进度条

- [ ] **Step 1: 加全局开关**

在 `crates/pan123-cli/src/cli.rs` 顶部 import 区加入：

```rust
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};
```

在 `Pan123Cli` 之上加入模块级开关与访问器：

```rust
/// 颜色与进度条总开关。进程启动时按 TTY 与 `--no-color` 决定一次。
static COLOR_ENABLED: AtomicBool = AtomicBool::new(true);

pub fn color_enabled() -> bool {
    COLOR_ENABLED.load(Ordering::Relaxed)
}

fn init_color_enabled(no_color_flag: bool) {
    let enabled = !no_color_flag && std::io::stdout().is_terminal();
    COLOR_ENABLED.store(enabled, Ordering::Relaxed);
}
```

在 `CliArgs` 中加入全局参数：

```rust
    /// 关闭彩色输出与进度条（stdout 不是终端时自动关闭）。
    #[arg(long, global = true)]
    no_color: bool,
```

- [ ] **Step 2: 初始化**

`Pan123Cli::run_from_env` 改为：

```rust
    pub fn run_from_env() -> Result<()> {
        let args = CliArgs::parse();
        init_color_enabled(args.no_color);
        let mut cli = Self {
            client: None,
            shell_state: Arc::new(Mutex::new(ShellState::new())),
        };
        match args.command {
            Some(Command::Shell) | None => cli.run_interactive(),
            Some(command) => cli.execute(command),
        }
    }
```

- [ ] **Step 3: 让颜色与进度条服从开关**

在 `cli.rs` 内定义一个包装函数，并把 26 处 `owo_colors` 调用改为经它转发：

```rust
/// 非 TTY 或显式 --no-color 时返回不施加任何颜色效果的字符串。
#[derive(Clone, Copy)]
pub struct Paint<'a>(pub &'a str);

impl<'a> Paint<'a> {
    pub fn green(self) -> String {
        if color_enabled() { self.0.green().to_string() } else { self.0.to_string() }
    }
    pub fn red(self) -> String {
        if color_enabled() { self.0.red().to_string() } else { self.0.to_string() }
    }
    pub fn yellow(self) -> String {
        if color_enabled() { self.0.yellow().to_string() } else { self.0.to_string() }
    }
    pub fn bright_cyan(self) -> String {
        if color_enabled() { self.0.bright_cyan().to_string() } else { self.0.to_string() }
    }
    pub fn bright_white(self) -> String {
        if color_enabled() { self.0.bright_white().to_string() } else { self.0.to_string() }
    }
    pub fn bright_blue(self) -> String {
        if color_enabled() { self.0.bright_blue().to_string() } else { self.0.to_string() }
    }
    pub fn bright_black(self) -> String {
        if color_enabled() { self.0.bright_black().to_string() } else { self.0.to_string() }
    }
    pub fn bright_yellow(self) -> String {
        if color_enabled() { self.0.bright_yellow().to_string() } else { self.0.to_string() }
    }
    pub fn dimmed(self) -> String {
        if color_enabled() { self.0.dimmed().to_string() } else { self.0.to_string() }
    }
    pub fn bold(self) -> String {
        if color_enabled() { self.0.bold().to_string() } else { self.0.to_string() }
    }
}
```

把每一处 `"文本".green()` 改写为 `Paint("文本").green()`。举三个真实例子（均在 `cli.rs`）：

```rust
// 改前
println!(
    "{} {} {}{}{}",
    "路径".bright_cyan().bold(),
    cwd.path.bright_white(),
    "(ID:".dimmed(),
    cwd.file_id.to_string().bright_black(),
    ")".dimmed()
);

// 改后（注意链式 .bold() 被丢弃）
println!(
    "{} {} {}{}{}",
    Paint("路径").bright_cyan(),
    Paint(&cwd.path).bright_white(),
    Paint("(ID:").dimmed(),
    Paint(&cwd.file_id.to_string()).bright_black(),
    Paint(")").dimmed()
);
```

```rust
// 改前
println!("{}", "登录成功。".green());
// 改后
println!("{}", Paint("登录成功。").green());
```

```rust
// 改前（格式化参数里直接调用）
println!("{} 已清理 {} 条上传会话记录", "完成".green(), removed.to_string().bright_yellow());
// 改后
println!("{} 已清理 {} 条上传会话记录", Paint("完成").green(), Paint(&removed.to_string()).bright_yellow());
```

逐处替换后跑 `cargo build`，编译器会逐一暴露遗漏处。

进度条：`CliProgress`（`cli.rs:1806`）定义如下，**不需要改结构体**：

```rust
struct CliProgress {
    multi: Arc<MultiProgress>,
    bars: Arc<Mutex<HashMap<String, ProgressBar>>>,
}
```

只需把 `callback()` 里创建进度条的那一处（`cli.rs:1831`）改为按开关选择，其余 `set_style` / `set_prefix` / `set_message` / `finish` 调用在隐藏进度条上都是空操作，无需改动：

```rust
                TransferEvent::Started {
                    id,
                    kind,
                    path,
                    total_bytes,
                } => {
                    // 非 TTY 或 --no-color 时使用隐藏进度条，后续调用逻辑不变。
                    let pb = if color_enabled() {
                        multi.add(ProgressBar::new(total_bytes.unwrap_or(0)))
                    } else {
                        ProgressBar::hidden()
                    };
                    pb.set_style(progress_style(total_bytes.is_some()));
                    // …其余保持原样
```

`callback()` 是被移动进闭包的，需在闭包外先求值一次：在 `fn callback(&self) -> ProgressCallback {` 之后加 `let enabled = color_enabled();`，闭包内改用 `enabled`（避免每次事件都读全局原子量）。

- [ ] **Step 4: 验证**

Run: `cargo build -p pan123-cli && cargo test --workspace && cargo clippy --all-features -- -D warnings`
Expected: 全部通过。

手工冒烟（管道 → 非 TTY → 应无颜色码）：

```powershell
$out = .\target\debug\pan123.exe --help 2>&1 | Out-String
if ($out -match "`e\[") { "FAIL: 仍有 ANSI 颜色码" } else { "OK: 无颜色码" }
.\target\debug\pan123.exe ls 2>&1 | Select-String "`e\[" | Measure-Object | Select-Object -ExpandProperty Count
```

Expected：第一条输出 `OK: 无颜色码`；第二条计数为 0。

再在真实终端里直接运行 `pan123 ls`，确认观感与改动前一致（有颜色）。

- [ ] **Step 5: 提交**

```bash
git add crates/pan123-cli/src/cli.rs
git commit -m "feat(cli): stdout 非 TTY 时自动去色，新增全局 --no-color"
```

---

## Phase 5：Skill 与分发

### Task 13: `skills/pan123-mcp/SKILL.md`

**Files:**
- Create: `skills/pan123-mcp/SKILL.md`

**Interfaces:**
- Consumes: Task 7-9 的工具清单与语义
- Produces: 无（文档）

- [ ] **Step 1: 写文件**

创建 `skills/pan123-mcp/SKILL.md`，内容如下（frontmatter 字段名必须是 `name` 与 `description`）：

```markdown
---
name: pan123-mcp
description: 通过已配置的 pan123 MCP 服务器管理 123 云盘。当用户要上传/备份文件到 123 云盘、从云盘下载、查找云端文件、整理云端目录、清理重复文件时使用。前提是 MCP 客户端已接入 pan123 MCP server。
---

# 123 云盘（pan123 MCP）

通过 pan123 MCP server 管理 123 云盘。**本 skill 假设客户端已配置 `pan123 mcp`**；
若工具列表里没有 `list_files` 等工具，说明没配好，改用 `pan123-cli` skill。

## 先检查登录

任何操作前先调用 `check_token`。返回"尚未登录"或"登录已失效"时，**不要**尝试自行登录——告诉用户在终端执行 `pan123 login` 扫码，然后重试。

## 核心概念：无状态

这个 server **没有"当前目录"**。每次调用都必须显式给出位置：

- 根目录的 `parent_id` 是 `0`
- 路径用 `resolve_path` 转成 file_id，例如 `resolve_path("/备份/2026")`
- 由 id 反查位置用 `get_path`
- 不要假设"刚才 ls 过的目录"就是下次调用的默认目录

文件 ID 可以传数字或字符串；工具返回的 id 一律是字符串。

## 工具速查

**浏览**：`list_files`（列目录）、`dir_tree`（递归树，必须给 `depth`）、`find_files`（按名搜索）、`get_file_info`（按 id 查详情）
**定位**：`resolve_path`（路径→id）、`get_path`（id→路径）
**账号**：`user_info`、`check_token`
**修改**：`create_folder`、`rename_file`、`move_files`、`copy_files`、`delete_files`
**传输**：`get_download_url`、`download_files`、`upload_file`、`upload_directory`、`list_upload_sessions`

## 工作流

### 备份目录到云盘

1. `resolve_path("/备份")` 拿到目标 id；目录不存在就 `create_folder` 建一个
2. 确认即将上传的本地路径是**绝对路径**（相对路径会被拒绝）
3. `upload_directory`，`duplicate` 用 `keep_both`（覆盖有风险，除非用户明确要求）
4. 汇报成功与失败数量；有失败项时列出失败文件

### 找文件

1. 知道大概位置就用 `find_files` 在该目录下搜
2. 知道完整路径就直接 `resolve_path`
3. 拿到结果后用 `get_path` 给出完整路径，方便用户核对

### 整理目录

1. `list_files` 看清现状，必要时 `dir_tree`
2. 把计划（哪些移到哪里、改成什么名）先讲给用户
3. **得到确认后**才调 `move_files` / `rename_file`

### 清理重复文件

1. `list_files` 或 `find_files` 取出候选
2. 按名称/大小分组，把「打算保留哪个、删除哪些」列成清单
3. **必须等用户明确确认**再调 `delete_files`

### 从云盘恢复文件

1. `resolve_path` / `find_files` 定位
2. 下载用 `download_files`，`dest_dir` 必须是绝对路径
3. 只要链接不落盘就用 `get_download_url`

## 安全规则

- `delete_files` 不可逆。**永远先列出将被删除的对象（名称 + id）并等待用户确认**，不要凭上下文推断用户想删什么。
- `move_files` / `rename_file` 前先展示变更前后对照。
- 上传前明确 `duplicate` 策略：默认 `keep_both`；用户要求覆盖时才用 `overwrite`。
- 不要在用户没要求的情况下执行删除或覆盖。

## 长任务

`upload_file` / `upload_directory` / `download_files` 是同步调用，可能需要很久，期间会推送进度通知。调用前先说明"这个可能要一会儿"。

**超大文件（数 GB）建议改走命令行**：MCP 通道在传输期间无法响应其他请求。让用户用 `pan123 upload`。

## 排错

| 现象 | 处置 |
|---|---|
| 提示需要登录 | 让用户在终端跑 `pan123 login`，不要自己尝试登录 |
| `resolve_path` 说找不到目录 | 用 `list_files` 从根目录逐层看，确认名称拼写（区分大小写与空格） |
| 参数报"必须是绝对路径" | 补全成绝对路径再重试 |
| 提示并发上限 | 等当前操作结束再重试，不要立刻重复调用 |
| 文件不存在（明确指出某个 id） | 用 `get_path` 确认该 id 现状，可能已被移动或删除 |
```

- [ ] **Step 2: 检查 frontmatter**

Run: `head -5 skills/pan123-mcp/SKILL.md`
Expected: 第一行 `---`，第二行 `name: pan123-mcp`，第三行 `description: ...`，第四行 `---`。`description` 必须包含触发场景关键词（上传/备份/下载/查找/整理/清理）。

- [ ] **Step 3: 提交**

```bash
git add skills/pan123-mcp/SKILL.md
git commit -m "docs(skills): 新增 pan123 MCP 使用指引"
```

---

### Task 14: `skills/pan123-cli/SKILL.md`

**Files:**
- Create: `skills/pan123-cli/SKILL.md`

**Interfaces:**
- Consumes: Task 11、12 的 `--json` 与去色行为
- Produces: 无（文档）

- [ ] **Step 1: 写文件**

创建 `skills/pan123-cli/SKILL.md`：

```markdown
---
name: pan123-cli
description: 用 pan123 命令行工具管理 123 云盘。当用户要上传/备份文件到 123 云盘、下载云端文件、查找或整理云端文件，但没有配置 pan123 MCP server，或明确要求用命令行时使用。
---

# 123 云盘（pan123 命令行）

前提：`pan123` 已在 PATH 中。先用 `pan123 --version` 确认。

**没有配置 MCP server 时用这个 skill；配了 MCP 就优先用 `pan123-mcp` skill（工具调用比拼命令行可靠）。**

## 铁律

- **绝不裸跑 `pan123`**：不带子命令会进入交互式 REPL 并永久挂住。永远带上具体子命令。
- **查询类命令一律加 `--json`**：`ls` / `tree` / `find` 支持，直接拿结构化数据，不要去解析人类可读表格。
- **不要跑 `pan123 login`**：它是交互式扫码，你代替不了。提示用户在终端自己执行。

## 认证

Token 来源优先级：环境变量 `PAN123_TOKEN` > 系统密钥环 > 本地文件。

- 想用特定账号：`PAN123_TOKEN=<token> pan123 ls --json`
- 检查登录状态：`pan123 status` 或直接跑 `pan123 ls --json`，失败信息会说明

## 命令速查

| 命令 | 作用 | `--json` |
|---|---|---|
| `pan123 ls [--parent <ref>] [--limit N]` | 列当前目录 | 支持 |
| `pan123 tree [--parent <ref>] [--depth N]` | 目录树 | 支持 |
| `pan123 find <关键词> [--parent <ref>] [--exact] [--dir-only] [--file-only]` | 搜索 | 支持 |
| `pan123 info` | 账号信息 | **本来就只有 JSON** |
| `pan123 status <ref> [--json]` | 文件详情 | 支持 |
| `pan123 pwd` / `pan123 cd <目录>` | 当前目录（CLI 有状态，MCP 没有） | 不支持 |
| `pan123 mkdir <名称> [--parent <ref>]` | 新建目录 | 不支持 |
| `pan123 rename <ref> <新名>` | 重命名 | 不支持 |
| `pan123 mv <ref...> --parent <ref>` | 移动 | 不支持 |
| `pan123 cp <ref...> --parent <ref>` | 复制 | 不支持 |
| `pan123 rm <ref...>` | 删除 | 不支持 |
| `pan123 upload <本地路径> [--parent <ref>] [--duplicate <策略>] [--jobs N] [--retries N]` | 上传文件或目录 | 不支持 |
| `pan123 download <ref...> [-d <目录>] [--retries N]` | 下载 | 不支持 |
| `pan123 upload-sessions [--clear] [--json]` | 未完成的上传会话 | 支持 |

`<ref>` 可以是文件/目录名、路径或 file_id，由 CLI 自行解析（**CLI 是有状态的**，依赖 `pan123 cd` 设过的当前目录）。

`--duplicate` 取值：`keep-both`（默认）/ `overwrite` / `cancel`。

## 工作流

### 备份目录到云盘

```bash
pan123 mkdir 备份 --parent /          # 若不存在；CLI 的根目录写作 /（不是 0）
pan123 upload /绝对/路径/项目 --parent 备份 --duplicate keep-both --retries 5
```

注意 CLI 与 MCP 对「根目录」的写法不同：CLI 用 `/`，`--parent 0` 会被当成
file_id 为 0 的真实查找并报 `NotFound`；MCP 工具才是 `parent_id = 0`。

上传耗时较长。退出码为 0 表示成功；非 0 时读 stderr 判断失败原因。

### 找文件

```bash
pan123 find "报告" --json
pan123 find "报告" --json --file-only
```

### 整理目录（先看清楚再动手）

```bash
pan123 ls --parent /备份 --json
pan123 mv 旧目录名 --parent /归档
```

### 清理重复

```bash
pan123 find "副本" --json
# 把要删的清单给用户确认后：
pan123 rm 副本1 副本2
```

### 下载恢复

```bash
pan123 download /备份/项目.zip -d /绝对/目标目录
```

## 注意事项

- **删除不可逆**：`rm` 前必须列出对象并取得用户确认，不要凭上下文推断。
- 输出**非 TTY 时自动去色**，所以管道里拿到的 `--json` 是干净 JSON。若仍有颜色码，加 `--no-color`。
- 长传输的进度条写 stderr，不会污染 stdout 的 JSON。
- 上传目录用 `--jobs N` 提高并发；Web 端协议对并发有限制，默认顺序上传更稳。
- 断点续传：中断后重跑相同命令会自动续传；要强制重来加 `--no-resume`（上传）。
```

- [ ] **Step 2: 检查 frontmatter 与命令一致性**

Run: `head -5 skills/pan123-cli/SKILL.md`
Expected: frontmatter 合法，`description` 中包含"未配置 MCP"这一区分条件。

再逐条核对命令名与参数确实存在于 `pan123 --help`：
Run: `.\target\debug\pan123.exe --help`
Expected: skill 中列出的每个子命令都出现在帮助里；`ls`/`tree`/`find` 的帮助中包含 `--json`。

- [ ] **Step 3: 提交**

```bash
git add skills/pan123-cli/SKILL.md
git commit -m "docs(skills): 新增 pan123 命令行使用指引"
```

---

### Task 15: 安装脚本与 README

**Files:**
- Modify: `install.sh`（末尾追加）
- Modify: `install.bat`
- Modify: `uninstall.sh`
- Modify: `uninstall.bat`
- Modify: `README.md`

**Interfaces:**
- Consumes: Task 13、14 建好的 `skills/` 目录
- Produces: 无（分发）

- [ ] **Step 1: install.sh**

在 `install.sh` 的"快速开始"提示之前插入：

```bash
# 安装 Claude Skill（可选）
SKILL_SRC_DIR="$(cd "$(dirname "$0")" && pwd)/skills"
SKILL_DEST_DIR="${HOME}/.claude/skills"

if [ -d "${SKILL_SRC_DIR}" ]; then
    echo ""
    read -p "是否一并安装 Claude Code Skill 到 ${SKILL_DEST_DIR}? (y/N): " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        mkdir -p "${SKILL_DEST_DIR}"
        cp -R "${SKILL_SRC_DIR}/pan123-mcp" "${SKILL_DEST_DIR}/"
        cp -R "${SKILL_SRC_DIR}/pan123-cli" "${SKILL_DEST_DIR}/"
        echo -e "${GREEN}✓ Skill 已安装到 ${SKILL_DEST_DIR}${NC}"
        echo ""
        echo -e "${YELLOW}接入 MCP 客户端：${NC}在客户端配置中加入"
        echo '  { "mcpServers": { "pan123": { "command": "pan123", "args": ["mcp"] } } }'
    fi
fi
```

- [ ] **Step 2: uninstall.sh**

在 `uninstall.sh` 删除二进制之后插入：

```bash
# 移除 Claude Skill
SKILL_DEST_DIR="${HOME}/.claude/skills"
for skill in pan123-mcp pan123-cli; do
    if [ -d "${SKILL_DEST_DIR}/${skill}" ]; then
        rm -rf "${SKILL_DEST_DIR}/${skill}"
        echo -e "${GREEN}✓ 已移除 Skill: ${skill}${NC}"
    fi
done
```

- [ ] **Step 3: install.bat 与 uninstall.bat**

`install.bat` 在输出"安装完成"之前插入（保持现有 `echo` 风格）：

```bat
REM 安装 Claude Skill（可选）
if exist "%~dp0skills" (
    echo.
    set /p INSTALL_SKILL=是否一并安装 Claude Code Skill 到 %USERPROFILE%\.claude\skills ? (y/N): 
    if /i "%INSTALL_SKILL%"=="y" (
        if not exist "%USERPROFILE%\.claude\skills" mkdir "%USERPROFILE%\.claude\skills"
        xcopy /E /I /Y "%~dp0skills\pan123-mcp" "%USERPROFILE%\.claude\skills\pan123-mcp" >nul
        xcopy /E /I /Y "%~dp0skills\pan123-cli" "%USERPROFILE%\.claude\skills\pan123-cli" >nul
        echo ✓ Skill 已安装
        echo.
        echo 接入 MCP 客户端，在配置中加入:
        echo   { "mcpServers": { "pan123": { "command": "pan123", "args": ["mcp"] } } }
    )
)
```

`uninstall.bat` 中对应加入：

```bat
REM 移除 Claude Skill
if exist "%USERPROFILE%\.claude\skills\pan123-mcp" rmdir /S /Q "%USERPROFILE%\.claude\skills\pan123-mcp"
if exist "%USERPROFILE%\.claude\skills\pan123-cli" rmdir /S /Q "%USERPROFILE%\.claude\skills\pan123-cli"
echo ✓ 已移除 pan123 Skill
```

- [ ] **Step 4: README**

在 README 的"特性"列表后新增一节：

```markdown
## 与 AI 客户端集成（MCP）

本工具内置 MCP（Model Context Protocol）服务器，可让你的 AI 客户端直接操作 123 云盘。

在客户端配置中加入：

```json
{
  "mcpServers": {
    "pan123": { "command": "pan123", "args": ["mcp"] }
  }
}
```

Claude Code 用户可以执行 `claude mcp add pan123 -- pan123 mcp` 完成配置。

配置后可用 18 个工具完成浏览、查找、整理、上传、下载等操作，传输过程会上报进度，且支持取消。

> MCP 服务器**无状态**：没有"当前目录"，每个操作都要显式指定 `parent_id`（根目录为 `0`）。

### Claude Skill

仓库 `skills/` 下提供两个 Skill：`pan123-mcp`（面向已配 MCP 的客户端）与 `pan123-cli`（面向命令行）。

运行 `./install.sh`（或 `install.bat`）时会提示是否一并安装到 `~/.claude/skills/`。也可手动复制：

```bash
mkdir -p ~/.claude/skills
cp -R skills/pan123-mcp skills/pan123-cli ~/.claude/skills/
```
```

- [ ] **Step 5: 验证并提交**

Run: `bash -n install.sh && bash -n uninstall.sh`
Expected: 无语法错误输出。

手工验证 bat 的引号与 `set /p` 语法无误（Windows 上执行 `install.bat` 到提示处按 N 退出即可）。

```bash
git add install.sh install.bat uninstall.sh uninstall.bat README.md
git commit -m "docs: 安装脚本与 README 增加 MCP 与 Skill 支持"
```

---

## 验收清单

全部任务完成后，逐条确认：

- [ ] `cargo test --workspace` 全绿
- [ ] `cargo clippy --all-features -- -D warnings` 无告警
- [ ] `cargo fmt --all -- --check` 无差异
- [ ] `pan123 mcp` 在无 token 时能启动，`tools/list` 返回 18 个工具
- [ ] `initialize` → `tools/list` → `tools/call` 全流程在真实客户端中连通
- [ ] 上传一个 >10MB 文件时客户端能看到进度通知
- [ ] 传输中取消，server 不返回响应且能继续接受新请求
- [ ] `pan123` 不带参数仍然进入交互 REPL（未被破坏）
- [ ] 管道中运行 `pan123 ls` 无 ANSI 颜色码；真实终端中颜色正常
- [ ] 两个 skill 目录能被 `install.sh` 正确复制
