---
name: pan123-cli
description: 通过 pan123 命令行工具操作 123 云盘（123pan）：登录、查看用户信息、浏览目录、上传/下载文件与目录、移动、复制、重命名、删除、搜索文件。Use when an agent needs to upload, download, back up, or manage files on the 123pan / 123 cloud drive, or when the user mentions 123盘、123云盘、pan123、网盘上传、网盘下载、云盘备份、云盘文件管理.
---

# pan123 命令行工具

`pan123` 是基于 123 云盘官方 Web API 的 Rust CLI，用于在服务器/无图形环境下上传备份数据、浏览与管理云盘文件。

## 何时使用

- 需要把本地文件/目录备份到 123 云盘，或从云盘下载到本地
- 需要列出、搜索、移动、复制、重命名、删除云盘文件
- 需要以脚本/自动化方式操作 123 云盘（在已登录的前提下）

## 不适用 / 边界

- **不支持安卓端协议，不支持破解每日下载限制**。基于官方 Web API，受会员与流量策略约束。
- **登录必须人工扫码**（见下），无法做到完全无人值守。
- 本项目无 crates.io 发布；二进制需本地构建或从 Releases 下载。

## 第 0 步：定位可执行文件

优先使用仓库内已构建的二进制，其次使用 PATH 中的 `pan123`：

```bash
PAN123=./target/release/pan123   # 仓库内构建产物
# PAN123=pan123                  # 已安装到 PATH 时
$PAN123 --version
```

若不存在则构建（需要网络与可写的 `~/.cargo`）：

```bash
cargo build --release      # 产物：target/release/pan123
```

## 第 1 步（关键）：确认登录状态

**在执行任何云盘操作前，先探测登录状态。** 所有需要鉴权的命令在 Token 缺失/失效时会**自动触发二维码登录并阻塞等待扫码**，在无人值守场景会挂起或失败。

```bash
$PAN123 info      # 成功：stdout 输出用户信息 JSON，退出码 0
                  # 失败：退出码 1，stderr 输出 error: ...
```

- 退出码 0 → 已登录，可继续。
- 退出码非 0（含 `authentication required` 等）→ 需要人工登录：

```bash
$PAN123 login     # 打印二维码，需用 123盘 App / 微信扫码；交互式
```

登录成功后 Token 存入**系统密钥链**（macOS Keychain / Windows Credential Manager / Linux Secret Service），后续命令无需再次登录。

### 凭据与配置目录

- **没有**用环境变量传递 Token 的方式。CLI 固定从密钥链读取，回退到状态文件 `123pan_token.json`（内容 `{"token": "...", "update_time": "..."}`）。
- 配置/状态目录优先级：`$PAN123_CONFIG_DIR` → `$APPDATA/pan123-cli` → `$USERPROFILE/.pan123-cli` → `./.pan123-cli`。
- 该目录存有 `123pan_cwd.json`（当前目录）与 `resume/`（下载续传元数据）。
- 无头环境替代方案（需谨慎，Token 为敏感凭据）：在可交互机器完成一次 `login`，或将 Token 写入 `$PAN123_CONFIG_DIR/123pan_token.json`，或直接使用 SDK `Pan123Client::new(Some(token))`。
- 域名可用环境变量覆盖（本地版本包含该支持时）：`PAN123_BASE_URL`（API 基址）、`PAN123_UCENTER_URL`（登录/ucenter 基址）。优先级高于服务端下发与内置默认值，可填裸域名（自动补 `https://`）。用于 DNS/代理异常时指定可用域名。

## 命令速查

| 命令 | 作用 | 关键参数 |
| --- | --- | --- |
| `login` | 二维码登录 / 检查登录 | 无 |
| `info` | 用户信息（**JSON**） | 无 |
| `pwd` | 显示当前目录（只读） | 无 |
| `cd <REF>` | 切换当前目录（**会持久化**） | `--id` 强制按 file_id |
| `ls` | 列出目录 | `-p/--parent <REF>`、`-l/--limit <N>`（默认 100） |
| `tree` | 目录树 | `-p/--parent <REF>`、`-d/--depth <N>`（默认 3；每层最多 500 项） |
| `upload <本地路径>` | 上传文件或整个目录 | `-p/--parent <REF>`、`--duplicate <策略>`、`--jobs <N>`、`--retries <N>`（默认 3） |
| `download <REF>...` | 下载 | `-d/--dir <本地目录>`（默认 `.`）、`--retries <N>` |
| `mkdir <名称>` | 新建目录 | `-p/--parent <REF>` |
| `rename <REF> <新名>` | 重命名 | 无 |
| `mv <目标目录> <源...>` | 移动 | 无 |
| `cp <目标目录> <源...>` | 复制 | 无 |
| `rm <REF>...` | 删除（**移入回收站**） | 无 |
| `status <REF>`（别名 `stat`） | 文件详情 | `--json` 输出 **JSON** |
| `find <关键词>` | 递归搜索 | `-p/--parent <REF>`、`-d/--depth <N>`（默认 5）、`--exact`、`--dir-only`、`--file-only` |
| `refresh` | 清理补全缓存 | `--all` 同时清理续传元数据 |
| `shell` | 交互式 REPL（Tab 补全） | **不要**在自动化中使用 |

> 注意：`status`/`stat` 是**查看单个文件详情**，不是查看 Token 状态（README 中"status 检查 Token"的说法已过时）。检查登录请用 `login` 或 `info`。帮助：`pan123 help`、`pan123 --help` 或 `pan123 <子命令> --help`。

## REF 引用语法

`REF` 可接受以下任一形式：

- `file_id` 数字，如 `123456`
- `id:123456`（显式 ID）
- 文件名，如 `report.pdf`（在当前工作目录下查找）
- 路径式，如 `/docs/report.pdf`、`a/b/c`
- 仅 `cd` 额外支持 `/`、`.`、`..`

## Agent 使用要点

1. **优先用 `-p/--parent` 或路径式 REF 显式定位，不要用 `cd`。** `cd` 会把当前目录写入全局状态文件，影响同一环境下**并发的其他 agent**，产生难以排查的耦合。
2. **结构化输出有限**：只有 `info` 和 `status --json` 输出 JSON。`ls`/`tree`/`find` 输出人类可读的彩色表格（含 emoji 图标），解析时需容错；如需机器可读列表，可靠做法是先用 `ls` 取"文件 ID"列，再对每个 ID 调用 `status <id> --json`，或直接使用 `pan123-sdk` 库。
3. **流分离与 ANSI**：结果与表格写 stdout；进度条写 stderr。重定向/抓取时只取 stdout。**输出始终包含 ANSI 转义序列**（即使管道/重定向，`NO_COLOR`、`CLICOLOR` 均无效），解析前先剥离：

   ```bash
   $PAN123 ls | sed $'s/\x1b\[[0-9;]*m//g'
   ```
4. **退出码**：成功 `0`；失败 `1`，错误信息为 stderr 上一行 `error: <消息>`。
5. **`upload` 的 `--duplicate` 默认 `keep-both`**（重复文件保留两者），可选 `overwrite` / `cancel`。备份覆盖场景需显式指定 `--duplicate overwrite`。
6. **多目标下载会打包**：`download a b c` 走批量下载接口，返回**单个下载链接**（目录同样如此），落盘为一个文件/压缩包，而不是多个独立文件。需要逐个下载时，请分多次调用。
7. **`--jobs`** 仅对目录上传生效（并发上传文件数）；单文件上传/下载为顺序执行，Web 端不宜并发。
8. **下载可断点续传**（`resume: true`），中断后重新执行同一命令即可继续；续传元数据在配置目录的 `resume/` 下。
9. **`rm` 移到回收站**、非永久删除，但仍属破坏性操作：批量 `rm`/`mv`/`cp` 前先向用户确认或先 `ls`/`status` 核验。

## 常见工作流

### 备份本地目录到云盘

```bash
# 1) 确认登录
$PAN123 info >/dev/null || { echo "需要先登录：$PAN123 login"; exit 1; }

# 2) 在云盘 /Backup 下创建以本地目录名命名的新目录并递归上传
$PAN123 upload /path/to/data -p /Backup --duplicate keep-both --jobs 4 --retries 5
```

### 下载文件到本地

```bash
# 先定位（可拿到 file_id）
$PAN123 find report --file-only
# 再下载到指定目录
$PAN123 download /docs/report.pdf -d ./downloads --retries 5
```

### 查看单个文件的机器可读详情

```bash
$PAN123 status 123456 --json
```

### 浏览与搜索

```bash
$PAN123 ls -p /                        # 列出根目录
$PAN123 tree -p /Backup -d 2           # 两层目录树
$PAN123 find "*.zip" -p /Backup        # 通配符搜索
$PAN123 find "re:^code-.*\.tar$"       # 正则（re: 前缀，首字符不能是 *）
```

## 故障排除

| 现象 | 原因 / 处理 |
| --- | --- |
| 命令卡在打印二维码 | Token 失效，`ensure_auth` 自动触发登录；需人工扫码 |
| `error: authentication required` | Token 缺失/过期，执行 `pan123 login` |
| `error: http error: error sending request for url (https://login.123pan.com/…)` | 域名连不上，**多为本地 DNS/代理劫持**（Clash 等的 fake-ip/分流规则把 `login.123pan.com`、`www.123pan.cn` 解析成黑洞 IP）。见下节"代理/DNS 环境" |
| `error: 网络异常，暂时无法验证登录状态` | 网络不可达；检查网络/代理/DNS 后重试 |
| `error: resource not found: <ref>` | REF 在当前目录下不存在；用 `ls`/`find` 确认名称或改用 `file_id` |
| 上传失败 | 提高 `--retries`、降低 `--jobs`（如 `--jobs 2`）后重试 |
| 下载中断 | 重新执行同一命令，自动从断点续传 |
| 下载受限 | 触发流量/会员限制（`isTrafficExceeded`）；本项目不绕过该限制 |

### 代理/DNS 环境（Clash 等）

代理工具常劫持 DNS，把 123 盘的登录/API 域名解析到黑洞或 fake-ip 地址，表现为 `error sending request`。**先验证**：

```bash
dig +short login.123pan.com   # 应为真实 IP，而非 172.31.255.254 / 198.20.2.x
```

让代理对 123 盘域名直连（Clash / mihomo 配置）：

```yaml
dns:
  fake-ip-filter:
    - "+.123pan.com"
    - "+.123pan.cn"
rules:
  - DOMAIN-SUFFIX,123pan.com,DIRECT
  - DOMAIN-SUFFIX,123pan.cn,DIRECT
```

或临时用环境变量指定可用域名后重试：

```bash
export PAN123_UCENTER_URL=user.123pan.cn
export PAN123_BASE_URL=www.123pan.cn
pan123 login
```

## 参考资料

- 命令细节、JSON 字段结构、SDK 用法见 [reference.md](reference.md)。
- 项目说明见 [README.md](../../README.md)。
