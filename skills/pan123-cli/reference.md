# pan123 参考手册

配套 [SKILL.md](SKILL.md) 使用的详细参考。

## 完整命令与参数

```text
pan123 <命令> [参数]

login                          二维码登录（Token 有效时仅提示状态）
info                           用户信息（JSON）
pwd                            当前目录
cd <REF> [--id]                切换目录；--id 将 REF 强制解析为 file_id
ls [-p|--parent <REF>] [-l|--limit <N>]        列目录（默认 limit 100）
tree [-p|--parent <REF>] [-d|--depth <N>]      目录树（默认 depth 3，每层最多 500 项）
upload <本地路径> [-p|--parent <REF>] [--duplicate keep-both|overwrite|cancel]
                 [--jobs <N>] [--retries <N>]  上传文件或目录（retries 默认 3）
download <REF>... [-d|--dir <目录>] [--retries <N>]   下载（dir 默认当前目录）
mkdir <名称> [-p|--parent <REF>]              新建目录
rename <REF> <新名称>                          重命名
mv <目标目录REF> <源REF>...                    移动
cp <目标目录REF> <源REF>...                    复制
rm <REF>...                                    删除（移入回收站）
status <REF> [--json]                          文件详情（别名 stat）
find <关键词> [-p|--parent <REF>] [-d|--depth <N>]
              [--exact] [--dir-only] [--file-only]   递归搜索
refresh [--all]                                清理补全缓存；--all 同时清续传元数据
shell                                          交互式 REPL（自动化请勿使用）
```

`help` 由 clap 自动生成，`pan123 help` 可用；也可用 `pan123 --help` 或 `pan123 <命令> --help`。在 `shell` 会话内 `help` 另有内置帮助。

## 搜索匹配规则（find）

| 输入 | 匹配方式 |
| --- | --- |
| `report` | 不区分大小写的子串匹配 |
| `*.zip`、`a?c` | 通配符（`*`、`?`），大小写敏感 |
| `re:^code-.*\.tar$` | 基础正则；`*` 不能出现在开头，且不支持连续 `**` |
| `--exact report.pdf` | 完全相等匹配 |

## 输出格式

### stdout / stderr 分离

- 表格与结果：stdout
- 进度条（indicatif）：stderr
- 错误：stderr，单行 `error: <消息>`；进程退出码 `1`

### JSON 输出

仅两处输出 JSON，字段由 serde 序列化，**键名为 PascalCase**：

`pan123 info` → `UserInfo`（服务端字段透传）：

```json
{ "Space": "...", "Nickname": "...", "Uid": 123, "...": "..." }
```

`pan123 status <REF> --json` → `FileInfo`：

```json
{
  "FileId": 123456,
  "ParentFileId": 0,
  "FileName": "report.pdf",
  "Type": 0,
  "Size": 40960,
  "Etag": "…（可能为 null）",
  "S3KeyFlag": "…（可能为 null）",
  "Status": "…（可能为 null）",
  "…": "服务端附加字段（extra，透传）"
}
```

字段说明：

| 字段 | 含义 |
| --- | --- |
| `FileId` | 文件/目录 ID（u64） |
| `ParentFileId` | 父目录 ID；根目录为 `0` |
| `FileName` | 名称 |
| `Type` | `0` 表示文件，非 `0` 表示目录 |
| `Size` | 字节数 |
| `Etag` / `S3KeyFlag` | 秒传/分片上传所需的服务端标识，可能为 null |
| `Status` | 服务端状态码，可能为 null |
| 其余键 | 服务端返回的附加字段，原样透传 |

### 人类可读输出（非结构化）

- `ls` 表格列：`图标 | 类型 | 名称 | 大小 | 文件 ID`（根目录/当前目录）
- `tree`：缩进树，每行 `└─ 图标 名称 (file_id)`
- `find` 表格列：`图标 | 类型 | 名称 | 路径 | ID`
- `status`（无 `--json`）：键值表（名称/类型/文件 ID/父目录 ID/大小/状态/Etag/路径/原始大小），若存在 extra 会再打印其 JSON
- `upload` 目录：打印「上传总结」（成功数、失败数与失败分类）
- `download`：打印 `已保存到 <本地路径> (<大小>)`
- 输出**始终**包含 ANSI 颜色与 emoji 图标（管道/重定向下依然输出；`NO_COLOR`、`CLICOLOR` 无效）。解析前先剥离 ANSI，例如 `sed $'s/\x1b\[[0-9;]*m//g'`；表格仍含 emoji 图标，需容错。

## 配置文件与状态

| 内容 | 路径 |
| --- | --- |
| 配置/状态根目录 | `$PAN123_CONFIG_DIR` → `$APPDATA/pan123-cli` → `$USERPROFILE/.pan123-cli` → `./.pan123-cli` |
| 当前目录 | `<根目录>/123pan_cwd.json`，内容 `{"file_id": <u64>, "path": "<string>"}` |
| 续传元数据 | `<根目录>/resume/<md5(目标路径)>.json` |
| Token（文件回退） | `<根目录>/123pan_token.json`，内容 `{"token": "<string>", "update_time": "<string>"}` |

Token 首选存储为系统密钥链（`keyring` crate：macOS Keychain / Windows Credential Manager / Linux Secret Service）；文件仅作回退。

> 兼容旧布局：读取时还会尝试 `./crates/pan123-cli/123pan_token.json` 与 `./crates/pan123-sdk/123pan_token.json`。

### 域名环境变量覆盖

| 变量 | 作用 | 默认（服务端下发前的内置值） |
| --- | --- | --- |
| `PAN123_BASE_URL` | API 基址 | `https://api.123278.com` |
| `PAN123_UCENTER_URL` | 登录/ucenter 基址 | `https://login.123pan.com` |

优先级：**环境变量 > 服务端下发的 `dydomain` > 内置默认值**。可填裸域名（自动补 `https://`），尾部斜杠会被去除。主要用于 DNS/代理异常时显式指定可用域名，例如：

```bash
export PAN123_UCENTER_URL=user.123pan.cn
export PAN123_BASE_URL=www.123pan.cn
```

域名发现会依次尝试 `login.123pan.com`、`user.123pan.cn`，单次请求 8s 超时；全部失败时保留默认值（此时可用上述环境变量覆盖）。

## 传输行为

### 上传

- 单文件：分片上传（16MB/片）、支持秒传（MD5 复用）、顺序执行。
- 目录：先在目标父目录下创建与本地目录同名的文件夹，再递归创建子目录并上传文件；`--jobs` 控制并发上传的文件数（默认 1，即顺序）。
- 重复策略 `--duplicate`：`keep-both`（默认）/ `overwrite` / `cancel`。
- 目录上传结束打印汇总；失败项按类型分类：`local-io`、`network`、`remote-api`、`conflict`、`validation`、`auth`、`unknown`。

### 下载

- 单文件走 `download_info`；多个目标或目录走 `batch_download_info` → 返回**单个下载链接**，落盘为一个文件/压缩包。
- 支持断点续传：临时文件 `<name>.part` + `<根目录>/resume/` 元数据；若 URL/ETag/Last-Modified/大小变化则自动重新开始。
- 目录下载：以 zip 形式返回（服务端行为）。

### 重试

默认 `max_attempts = 3`，指数退避（base 750ms，上限 30s，含 20% 抖动）。`--retries` 覆盖 `max_attempts`。

## 错误语义（退出码均为 1）

`error: <Pan123Error>` 的常见形式：

| 消息 | 含义 |
| --- | --- |
| `authentication required` | 未登录 / Token 失效 |
| `api error <code>: <message>` | 服务端返回错误 |
| `resource not found: <ref>` | REF 不存在 |
| `invalid path: <ref>` | 路径/REF 非法 |
| `网络异常，暂时无法验证登录状态：无法连接 <url>…` | 校验登录时网络不可达；提示检查 DNS/代理 |
| `无法连接 <url>：<reqwest 错误>…` | 登录域名连接失败，多为 DNS/代理劫持；见 SKILL.md「代理/DNS 环境」 |
| `operation failed: …` | 其它操作失败（含“<名称> 不是目录”） |
| `file conflict: <path>` | 文件冲突 |
| `insufficient storage space` | 空间不足 |
| `rate limit exceeded, retry after <n>s` | 触发限流 |

可重试类错误：IO、HTTP、超时。

## 作为库使用（Rust）

```toml
[dependencies]
pan123-sdk = { path = "crates/pan123-sdk" }
```

```rust
use pan123_sdk::{Pan123Client, DuplicateMode};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 传入已有 token；或 None，从密钥链/状态文件读取
    let mut client = Pan123Client::new(None)?;

    // 需要时走二维码登录（交互式）
    // client.login_by_qrcode()?;

    for file in client.get_file_list(0, 1, 100)? {
        println!("{}: {}", file.file_id, file.file_name);
    }

    let info = client.upload_file("test.txt", 0, DuplicateMode::KeepBoth)?;
    println!("上传成功: {}", info.file_name);
    Ok(())
}
```

导出的主要类型：`Pan123Client`、`TokenCheckStatus`、`Pan123Error`、`FileInfo`、`DuplicateMode`、`TransferOptions`、`UploadOptions`、`DownloadOptions`、`RetryPolicy`、`UploadDirectoryReport`、`UploadFailureKind`、`TransferEvent`、`SecureStorage`、`RateLimiter`。

## 免责声明

本项目基于 123 云盘官方 Web 端协议实现，仅供学习交流。不支持安卓端协议，不支持绕过每日下载限制与流量策略。请遵守服务条款。
