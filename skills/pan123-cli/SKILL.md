---
name: pan123-cli
description: 用 pan123 命令行工具管理 123 云盘。当用户要上传/备份文件到 123 云盘、下载云端文件、查找或整理云端文件，但没有配置 pan123 MCP server，或明确要求用命令行时使用。
---

# 123 云盘（pan123 命令行）

前提：`pan123` 已在 PATH 中。先用 `pan123 --version` 确认。

**没有配置 MCP server 时用这个 skill；配了 MCP 就优先用 `pan123-mcp` skill（工具调用比拼命令行可靠）。**

## 铁律

- **绝不裸跑 `pan123`**：不带子命令会进入交互式 REPL 并永久挂住。永远带上具体子命令。
- **发命令前先建立登录态**：未登录时带鉴权的子命令**不会干净报错**，而是自动进入交互式扫码登录，最长卡约 3 分钟才返回 `二维码登录超时`。见下方「认证」。
- **查询类命令一律加 `--json`**：`ls` / `tree` / `find` 支持，直接拿结构化数据，不要去解析人类可读表格。
- **输出被捕获时用 `--no-color`**：stdout 不是终端时会自动去色；显式加全局 `--no-color`（子命令前后皆可）可确保拿到的 `--json` 永远是干净 JSON，并同时关闭进度条。
- **不要跑 `pan123 login`**：它是交互式扫码，你代替不了。提示用户在终端自己执行。

## 认证

Token 来源优先级：环境变量 `PAN123_TOKEN` > 系统密钥环 > 本地文件。

**先建立登录态，再发需要鉴权的命令。** 两条正路：

- 用环境变量传 token（优先级最高，且不读、不写本机已存的登录态）：`PAN123_TOKEN=<token> pan123 ls --json`
- 请用户在**他自己的终端**里执行一次 `pan123 login` 扫码登录；登录态会保存下来，供后续命令复用。

没有专用的「查登录状态」子命令：`pan123 status` 需要给出具体文件（`<TARGET>` 是必填位置参数），并不是登录检查；而任何需要鉴权的命令在未登录时都会先卡进扫码流程（约 3 分钟）才报错。

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
| `pan123 mv <目标目录> <ref...>` | 移动 | 不支持 |
| `pan123 cp <目标目录> <ref...>` | 复制 | 不支持 |
| `pan123 rm <ref...>` | 删除 | 不支持 |
| `pan123 upload <本地路径> [--parent <ref>] [--duplicate <策略>] [--jobs N] [--retries N]` | 上传文件或目录 | 不支持 |
| `pan123 download <ref...> [-d <目录>] [--retries N]` | 下载 | 不支持 |
| `pan123 upload-sessions [--clear] [--json]` | 未完成的上传会话 | 支持 |

`<ref>` 可以是文件/目录名、路径或 file_id，由 CLI 自行解析（**CLI 是有状态的**，依赖 `pan123 cd` 设过的当前目录）。

注意 `mv` / `cp` 的**目标目录是第一个位置参数**（没有 `--parent` 标志）：`pan123 mv <目标目录> <ref...>`。

`--duplicate` 取值：`keep-both`（默认）/ `overwrite` / `cancel`。

## 工作流

### 备份目录到云盘

```bash
pan123 mkdir 备份 --parent 0          # 若不存在
pan123 upload /绝对/路径/项目 --parent 备份 --duplicate keep-both --retries 5
```

上传耗时较长。退出码为 0 表示成功；非 0 时读 stderr 判断失败原因。

### 找文件

```bash
pan123 find "报告" --json
pan123 find "报告" --json --file-only
```

### 整理目录（先看清楚再动手）

```bash
pan123 ls --parent /备份 --json
pan123 mv /归档 旧目录名
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
- 输出**非 TTY 时自动去色**，所以管道里拿到的 `--json` 是干净 JSON。若仍有颜色码，加全局 `--no-color`（它同时关闭进度条）。
- 长传输的进度条写 stderr，不会污染 stdout 的 JSON。
- 上传目录用 `--jobs N` 提高并发；Web 端协议对并发有限制，默认顺序上传更稳。
- 断点续传：中断后重跑相同命令会自动续传；要强制重来加 `--no-resume`（上传）。
