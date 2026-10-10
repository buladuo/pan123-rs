# pan123-rs：MCP 服务与 Claude Skill 设计

日期：2026-10-10
状态：已通过设计评审，待写实施计划

## 1. 背景与目标

`pan123-rs` 目前是一个 Rust workspace，包含同步阻塞的 `pan123-sdk` 和基于 clap 的
命令行工具 `pan123-cli`（二进制名 `pan123`）。用户要通过 AI 客户端管理 123 云盘，
只能让模型拼命令行，既不可靠也难以覆盖多步流程。

本设计为项目增加两个面向**所有用户**发布的能力：

1. **MCP 服务**：新增 `pan123 mcp` 子命令，把云盘操作以 MCP（Model Context
   Protocol）工具的形式暴露给任意 MCP 客户端（Claude Code、Cursor、Cline 等）。
2. **两套 Skill**：随仓库分发两个 Claude Skill，一套服务 MCP 通道，一套包装
   CLI 通道，各自覆盖完整使用场景。

**成功标准**：用户装了 `pan123` 就能一行配置接入 MCP 客户端；没配 MCP 的用户
也能靠 CLI Skill 完成任务；两条通道都能覆盖备份、恢复、查找、整理、清理重复
五类工作流。

## 2. 关键设计决策

| 决策点 | 选择 | 理由 |
|---|---|---|
| 目标用户 | 所有项目用户 | 决定协议严格度与文档投入 |
| MCP 实现 | 手写 stdio JSON-RPC | 全仓库零 async，不引入 tokio；工具面小 |
| 部署形态 | `pan123 mcp` 子命令 | 单一二进制，安装体验最好，零新增依赖 |
| 工具范围 | 全量暴露（18 个） | 完整能力，破坏性操作由客户端权限层拦截 |
| 状态模型 | 无状态，参数显式传 | 长驻进程 + 共享状态文件是并发陷阱 |
| 长任务 | 同步返回 + progress 通知 | 模型拿到确定结果，同时可见进度 |
| 并发模型 | 读线程 + 每调用工作线程 + 写线程 | 不阻塞、可取消、可并发（见 §7） |
| Skill 关系 | 两套独立 Skill | 避免单个 Skill 维护两条等价路径 |
| Skill 分发 | 随 install 脚本安装 | 不绑定单一客户端 |
| CLI 输出 | 补 `--json` + 非 TTY 去色 | 让 CLI Skill 能稳定解析 |

## 3. 范围

### 做

- `pan123 mcp` 子命令，stdio 传输，18 个 MCP 工具
- 传输进度通知（`notifications/progress`）
- 请求取消（`notifications/cancelled`）真正中断传输
- SDK 增加取消支持（`TransferOptions.cancel` + `Pan123Error::Cancelled`）
- CLI 补 `--json`（`ls`/`tree`/`find`/`info`）与全局 `--no-color` / 非 TTY 自动去色
- `skills/pan123-mcp/SKILL.md`、`skills/pan123-cli/SKILL.md`
- `install.sh` / `install.bat` / `uninstall.*` 集成 Skill 安装
- README 增补 MCP 配置章节

### 不做

- Streamable HTTP 传输（只做 stdio）
- MCP 的 resources / prompts / sampling / elicitation
- `outputSchema` / `structuredContent`（v1 只给 `content`，将来加是纯增量）
- 引入 `tokio` 或 `rmcp`
- 暴露 `login`（二维码无法在 stdio 交互）、`pwd`/`cd`（无状态）、
  `shell`/`clear`/`refresh`（交互态或无意义）
- Claude Code Plugin / marketplace 分发

## 4. 架构

```
MCP 客户端 (Claude Code / Cursor / …)
   │  stdio：换行分隔 JSON-RPC 2.0
   ▼
pan123 mcp                    ← pan123-cli 新增子命令
   ├── mcp/protocol.rs        ← 分帧 + 方法分发（不认识 SDK）
   └── mcp/tools.rs           ← 18 个工具的 schema + 参数映射 → Pan123Client
              │
              ▼  pan123-sdk（除取消支持外不动，不引入 tokio）
```

**关键边界**：`protocol.rs` 面向一个 `trait ToolHandler`（列出工具 / 调用工具），
完全不认识 `Pan123Client`。`tools.rs` 实现该 trait 并持有 `Pan123Client`。这样协议层
可以用假 handler 完整测试，无需打网络，也避免为测试去给 `Pan123Client` 抽 trait
（它当前没有抽象层）。

**新增依赖：零。** `serde_json` 已在依赖中。

**无状态**：不提供 `cd`/`pwd`，不读写 `123pan_cwd.json`。目录一律靠 `parent_id`
显式传递，根目录为 `0`。

### stdout 纯净性（硬约束）

MCP stdio 传输规定 server 的 stdout **只允许**出现 MCP 消息。经排查：

- `pan123-sdk` 中仅 `client.rs:1481`、`client.rs:1513` 使用 `println!`，均位于
  `login_by_qrcode`。该函数不通过 MCP 暴露，因此当前无冲突。
- 其余输出（`client.rs:810/837/920/1592-1625`、`secure_storage.rs:255`）均为
  `eprintln!`/`eprint!`，走 stderr，符合规范（stderr 允许用于日志）。
- `pan123-cli` 的彩色输出与 indicatif 进度条只用于 CLI 路径，MCP 路径不复用。

**约束**：今后若把 `login_by_qrcode` 接入 MCP，必须先消除其 stdout 输出。
MCP 路径下任何 `println!` 都是协议级 bug。

**已知噪音（接受）**：`client.rs:1590` 在文件 >500MB 时向 stderr 打印哈希进度条。
不影响协议，客户端可忽略。

## 5. 协议层（mcp/protocol.rs）

### 分帧

stdin 逐行读取（`BufRead::read_line`），一行一个 JSON-RPC 消息；响应写 stdout
一行 + `\n` 并立即 flush。消息内不得含换行。

### 生命周期

1. `initialize` → 返回 `InitializeResult`：
   - `protocolVersion`：客户端请求的版本若在支持列表内则原样回显，否则回本实现
     最新的 `2025-06-18`。支持列表：`2025-06-18`、`2025-03-26`、`2024-11-05`。
   - `capabilities`：`{ "tools": {} }`
   - `serverInfo`：`{ "name": "pan123", "version": env!("CARGO_PKG_VERSION") }`
2. `notifications/initialized` → 无响应
3. `tools/list`、`tools/call`、`ping`
4. stdin EOF → 干净退出（客户端主动关闭，不是错误）

### 方法分发与错误码

| 情形 | 处理 |
|---|---|
| 未知方法且带 `id` | `-32601` |
| 参数不合法 | `-32602` |
| JSON 解析失败 | `-32700` |
| 内部错误 | `-32603` |
| 未知通知（无 `id`） | 静默忽略（规范要求） |

`initialize` 之前的请求（`initialize` 本身除外）返回 `-32600`。

### 返回结构

```json
{ "content": [ { "type": "text", "text": "<人类可读>" } ], "isError": false }
```

`inputSchema` 正常声明；v1 不声明 `outputSchema` / `structuredContent`。

### 文件 ID 的数值安全

123 云盘的文件 ID 是 `u64`，可能超过 JavaScript 的 `2^53` 安全整数范围。
因此：**所有 id 类参数接受 number 或 string 两种形式；所有输出一律序列化为
string。** 这避免 JS 客户端静默丢精度。

### 本地路径语义

`upload_file` / `upload_directory` 的源路径、`download_files` 的目标目录，
都相对于 **MCP server 进程的工作目录**，而该目录由客户端启动进程时决定，
模型无从可靠推断。

处理方式：这些参数在 `inputSchema` 与工具描述中明确要求**绝对路径**；
`download_files` 显式提供 `dest_dir` 参数，缺省时报 `isError` 要求模型给出，
不做隐式落盘。工具描述中注明"路径相对于 server 进程工作目录"。

## 6. 工具清单（18 个）

| 工具 | 对应 SDK | 说明 |
|---|---|---|
| `list_files` | `get_file_list` | 列目录 |
| `dir_tree` | 递归 `get_file_list` | 需显式 `depth`，防递归爆炸 |
| `find_files` | CLI `find` 逻辑 | 按名搜索 |
| `get_file_info` | `get_file_info` | 按 id 批量查 |
| `resolve_path` | `find_child_folder_by_name` 逐段走 | 路径 → file_id |
| `get_path` | `get_path_chain` | file_id → 路径，便于模型向用户复述位置 |
| `user_info` | `get_user_info` | |
| `check_token` | `check_token_status` | 登录态是否有效 |
| `create_folder` | `create_folder` | |
| `rename_file` | `rename_file` | 破坏性 |
| `move_files` | `move_files` | 破坏性 |
| `copy_files` | `copy_files` | |
| `delete_files` | `delete_files` | 破坏性 |
| `get_download_url` | `get_download_link` | 只取链接，不落盘 |
| `download_files` | `download_files_with` | 长任务，带进度 |
| `upload_file` | `upload_file_with` | 长任务，带进度 |
| `upload_directory` | `upload_directory_with` | 长任务，带进度 |
| `list_upload_sessions` | `upload_session` | 断点续传会话 |

工具名与参数名统一 snake_case。

**破坏性操作**按"全量暴露"直接提供，不额外加开关。权限确认交给 MCP 客户端
（Claude Code 默认会拦截工具调用）。安全引导写在 Skill 中（先列影响清单、
得到确认再执行）。

## 7. 并发与取消（方案 A）

```
主线程          read stdin 逐行 → 解析
                ├─ initialize/tools/list/ping  → 就地回
                ├─ tools/call                  → 建取消令牌，spawn 工作线程
                └─ notifications/cancelled     → 查 id，置位令牌   ← 不阻塞
                                                      │
工作线程（每调用一个）                            │
   Pan123Client::clone()（共享限流器）             │
   跑传输，ProgressCallback → 节流 → 写通道 ────────┤
                                                  ▼
写线程          独占 stdout，从 mpsc 收消息，串行写出
```

设计依据（已核对代码）：

- `Pan123Client` 字段全 Send+Sync 且 `Clone`，可跨线程共享。
- `RateLimiter` 内部是 `Arc<Mutex<RateLimiterInner>>`，clone 后**共享**限流状态，
  多线程不会绕过限流。
- `Pan123Error` 当前**没有** `Cancelled` 变体，需新增。

### 取消链路

客户端发 `notifications/cancelled` → 置位该请求的 `Arc<AtomicBool>` → 传输循环
检出 → 返回 `Pan123Error::Cancelled` → **不回响应**（MCP 规范：被取消的请求
不应有响应）。

### SDK 改动（3 处，唯一触及 pan123-sdk 的改动）

1. `error.rs`：新增 `Cancelled` 变体。**必须不加入 `is_retryable()`** ——
   否则 `retry_with_backoff` 会重跑已取消的操作，取消失效。
2. `transfer.rs`：`TransferOptions` 新增 `cancel: Option<Arc<AtomicBool>>`，
   `Default` 为 `None` —— CLI 现有调用路径行为完全不变
   （需补 `use std::sync::atomic::AtomicBool`）
3. `client.rs`：下载读循环（约 601 行 `loop {`）与上传分片循环各插入一处检查点，
   检出置位即提前返回 `Err(Pan123Error::Cancelled)`；上传侧还需通知已在
   处理其它分片的工作线程尽快退出

### 健壮性要求

- **工作线程体必须用 `catch_unwind` 包住**：否则一次 panic 会让客户端永久挂住，
  等待一个永不到来的响应。panic 也要回一条 `isError` 响应。
- **并发上限 4**：超出直接回 `isError`，防止线程爆炸。
- 所有输出经 mpsc 交给写线程，工作线程**绝不直接写 stdout**。

## 8. 进度通知

客户端在 `tools/call` 的 `params._meta.progressToken` 提供令牌，给了才发：

`TransferEvent::Progress` → `notifications/progress`，params 为
`{ progressToken, progress: <bytes>, total?, message? }`。

**节流**：按「字节变化 ≥1% 或距上次 ≥500ms」发送。否则 1GB 文件会产生数十万条
通知淹没客户端。

没有 `progressToken` 时回调直接丢弃——**绝不能退化成往 stdout 打印**。

`TransferEvent` 的 `Started`/`Retrying`/`Finished`/`Failed` 映射为 `message`
字段的变化，同样受节流约束。

## 9. 错误处理

- **工具级失败用 `isError: true`**，不用 JSON-RPC error。这是 MCP 惯例：模型能
  看到错误内容并自行纠正。
- `Pan123Error` 映射为可操作的人话：
  - token 缺失/失效 → "请先在终端运行 `pan123 login`"
  - 文件不存在 → 明确指出是哪个 id
  - 网络错误 → 建议重试
  - `Cancelled` → 不返回响应
- **token 缺失时不要退出**：server 照常启动，`tools/list` 正常返回，调用工具时
  才返回 `isError` + 登录指引。否则客户端只看到"MCP server 挂了"，用户无从下手。

## 10. CLI 增强

### `--json`

照搬 `status` / `upload-sessions` 已有模式（`print_status` 中 `json_output` 分支），
为 `ls`、`tree`、`find`、`info` 补 `--json`，输出完整结构体而非人类可读表格。

### 去色

新增全局 `--no-color`（clap `global = true`，任意子命令均可用）。

判定规则：**stdout 不是 TTY** 时自动关闭颜色与进度条，等价于隐式 `--no-color`；
`--no-color` 显式给出时无条件关闭。进程内以一个一次性初始化的全局开关承载，
`cli.rs` 中 26 处颜色调用与 indicatif 进度条统一改为查询该开关（不改变默认
终端下的观感）。

现状是全仓库无任何 TTY 检测，`owo-colors` 的 `.green()` 无条件着色，
`NO_COLOR` 不生效。检测用 `std::io::IsTerminal`（标准库，无需新依赖）。

### Skill 相关注意

`pan123` 不带参数会进入交互 REPL（`Command::Shell | None => run_interactive`），
模型绝不能这样调用，Skill 中需明确警示。

## 11. 两套 Skill

### `skills/pan123-mcp/SKILL.md`

面向已配置 MCP 的用户。内容：

1. 前置检查：先 `check_token`，失效则指引去终端 `pan123 login`
2. **无状态模型**：根目录 `parent_id=0`；路径↔id 用 `resolve_path` / `get_path`；
   强调不能假设存在"当前目录"
3. 工具速查表：按浏览 / 定位 / 传输 / 修改四组
4. 工作流：备份、找文件、整理目录、清理重复、下载恢复
5. 安全规则：`delete_files`/`move_files`/`rename_file` 先列影响清单再确认；
   上传必须明确 `duplicate` 策略
6. 长任务：说明进度通知；超大文件转 CLI
7. 排错表：`isError` 常见情形 → 处置动作

### `skills/pan123-cli/SKILL.md`

面向未配 MCP 或明确要用命令行的用户。内容：

1. 前提：`pan123` 在 PATH；**绝不裸跑 `pan123`**（会进 REPL 并挂住）
2. 输出约定：一律加 `--json`
3. 命令速查（含参数与 `--json` 支持情况）
4. 同一批五类工作流，用命令表达
5. 认证：`PAN123_TOKEN` > 系统密钥环 > 文件；`login` 是交互式扫码，模型代替不了
6. 注意事项：退出码、错误走 stderr、长传输的观测方式

两套 Skill 在 frontmatter 的 `description` 中区分触发条件，避免同时命中。

## 12. 分发

`skills/` 随仓库走。`install.sh` / `install.bat` 增加一步：把两个 skill 目录拷贝到
`~/.claude/skills/`；`uninstall.sh` / `uninstall.bat` 对应清理。README 增补 MCP
配置章节，给出客户端配置片段：

```json
{ "mcpServers": { "pan123": { "command": "pan123", "args": ["mcp"] } } }
```

## 13. 测试策略

| 目标 | 测试内容 |
|---|---|
| `protocol.rs` | 假 handler 驱动：分帧、`initialize` 版本协商、未知方法 `-32601`、通知不回响应、`isError` 结构、EOF 干净退出、`initialize` 前请求被拒 |
| 取消 | 慢速假 handler + 发 `cancelled`，断言不回响应且线程退出 |
| 并发 | 并发上限触发时返回 `isError` |
| `tools.rs` | 工具名唯一性、`inputSchema` 合法性自检、参数映射纯函数部分、id 接受 number 与 string |
| SDK | `TransferOptions.cancel` 中断下载、中断上传各一例 |
| CLI | `--json` 字段完备、非 TTY 无颜色码 |

不打真实网络。需要网络的验证走手工冒烟。

## 14. 文件清单

**新增**

- `crates/pan123-cli/src/mcp/mod.rs`
- `crates/pan123-cli/src/mcp/protocol.rs`
- `crates/pan123-cli/src/mcp/tools.rs`
- `skills/pan123-mcp/SKILL.md`
- `skills/pan123-cli/SKILL.md`

**修改**

- `crates/pan123-sdk/src/error.rs`（`Cancelled` 变体）
- `crates/pan123-sdk/src/transfer.rs`（`TransferOptions.cancel`）
- `crates/pan123-sdk/src/client.rs`（两处取消检查点）
- `crates/pan123-cli/src/cli.rs`（`mcp` 子命令、`--json`、`--no-color`）
- `install.sh`、`install.bat`、`uninstall.sh`、`uninstall.bat`
- `README.md`
