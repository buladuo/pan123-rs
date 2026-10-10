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
