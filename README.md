# pan123-rs

基于 Rust 实现的 123 盘命令行工具，支持文件上传、下载、管理等功能。

## 特性

- 🚀 高性能：Rust 实现，支持多线程并发上传下载
- 📦 断点续传：支持大文件断点续传
- 🔐 安全存储：使用系统密钥链安全存储 Token
- 🎨 友好界面：彩色输出、进度条、表格展示
- 🔄 自动重试：网络错误自动重试，支持指数退避
- 📊 限流控制：可配置的 API 请求限流
- 🌲 目录树：支持递归显示目录结构
- 🔍 文件搜索：支持按名称搜索文件

## 安装

### 从源码编译

#### 前置要求

- Rust 1.75 或更高版本
- Cargo

#### Linux / macOS

```bash
# 克隆仓库
git clone https://github.com/yourusername/pan123-rs.git
cd pan123-rs

# 编译安装
./scripts/build.sh

# 或者手动编译
cargo build --release
sudo cp target/release/pan123 /usr/local/bin/
```

#### Windows

```powershell
# 克隆仓库
git clone https://github.com/yourusername/pan123-rs.git
cd pan123-rs

# 编译安装
.\scripts\build.ps1

# 或者手动编译
cargo build --release
# 将 target\release\pan123.exe 添加到 PATH
```

### 预编译二进制文件

从 [Releases](https://github.com/yourusername/pan123-rs/releases) 页面下载对应平台的二进制文件。

## 快速开始

### 1. 登录

首次使用需要登录获取 Token：

```bash
pan123 login
```

使用 123 盘 App 或微信扫描二维码完成登录。

### 2. 查看用户信息

```bash
pan123 info
```

### 3. 浏览文件

```bash
# 列出当前目录文件
pan123 ls

# 显示目录树
pan123 tree

# 切换目录
pan123 cd 文件夹名称

# 显示当前路径
pan123 pwd
```

### 4. 上传文件

```bash
# 上传单个文件
pan123 upload /path/to/file.txt

# 上传整个目录
pan123 upload /path/to/directory

# 指定上传到特定目录（使用文件 ID）
pan123 upload file.txt --parent 123456

# 设置重复文件处理策略
pan123 upload file.txt --duplicate overwrite  # 覆盖
pan123 upload file.txt --duplicate keep-both  # 保留两者
pan123 upload file.txt --duplicate cancel     # 取消

# 设置并发数和重试次数
pan123 upload large-dir --jobs 4 --retries 5
```

### 5. 下载文件

```bash
# 下载单个文件（使用文件名或 ID）
pan123 download file.txt

# 下载多个文件
pan123 download file1.txt file2.txt 123456

# 指定下载目录
pan123 download file.txt --dir /path/to/save

# 设置重试次数
pan123 download file.txt --retries 5
```

### 6. 文件管理

```bash
# 创建文件夹
pan123 mkdir 新文件夹

# 重命名文件
pan123 rename old-name.txt new-name.txt

# 移动文件
pan123 mv 目标文件夹 file1.txt file2.txt

# 复制文件
pan123 cp 目标文件夹 file1.txt file2.txt

# 删除文件
pan123 rm file1.txt file2.txt

# 搜索文件
pan123 find keyword
```

### 7. 交互式 Shell

启动交互式命令行界面：

```bash
pan123 shell
```

在 Shell 中可以使用所有命令，支持 Tab 补全和历史记录：

```
pan123> ls
pan123> cd Documents
pan123> upload file.txt
pan123> exit
```

## 命令详解

### login - 登录

```bash
pan123 login
```

使用二维码扫码登录，Token 会自动保存到系统密钥链。

### info - 用户信息

```bash
pan123 info
```

显示当前登录用户的详细信息。

### pwd - 当前路径

```bash
pan123 pwd
```

显示当前工作目录的路径和 ID。

### cd - 切换目录

```bash
# 使用文件名切换
pan123 cd Documents

# 使用文件 ID 切换
pan123 cd 123456 --id

# 返回根目录
pan123 cd /

# 返回上级目录
pan123 cd ..
```

### ls - 列出文件

```bash
# 列出当前目录
pan123 ls

# 列出指定目录（使用文件名）
pan123 ls --parent Documents

# 列出指定目录（使用文件 ID）
pan123 ls --parent 123456

# 限制显示数量
pan123 ls --limit 50
```

### tree - 目录树

```bash
# 显示当前目录树（默认深度 3）
pan123 tree

# 指定深度
pan123 tree --depth 5

# 显示指定目录的树
pan123 tree --parent Documents
```

### upload - 上传

```bash
pan123 upload <本地路径> [选项]

选项：
  -p, --parent <ID>           上传到指定父目录 ID
      --duplicate <策略>      重复文件处理：cancel, keep-both, overwrite
      --jobs <数量>           并发上传线程数
      --retries <次数>        失败重试次数（默认 3）
```

### download - 下载

```bash
pan123 download <文件名或ID...> [选项]

选项：
  -d, --dir <目录>            保存到指定目录（默认当前目录）
      --retries <次数>        失败重试次数（默认 3）
```

### mkdir - 创建文件夹

```bash
pan123 mkdir <文件夹名> [选项]

选项：
  -p, --parent <ID>           在指定父目录下创建
```

### rename - 重命名

```bash
pan123 rename <文件名或ID> <新名称>
```

### mv - 移动文件

```bash
pan123 mv <目标目录> <源文件...>

# 示例
pan123 mv Documents file1.txt file2.txt
pan123 mv 123456 file.txt  # 使用目标目录 ID
```

### cp - 复制文件

```bash
pan123 cp <目标目录> <源文件...>

# 示例
pan123 cp Backup file1.txt file2.txt
```

### rm - 删除文件

```bash
pan123 rm <文件名或ID...>

# 示例
pan123 rm file1.txt file2.txt
pan123 rm 123456 789012  # 使用文件 ID
```

### find - 搜索文件

```bash
pan123 find <关键词>

# 示例
pan123 find report
pan123 find .pdf
```

### status - Token 状态

```bash
pan123 status
```

检查当前 Token 的有效性。

### stat - 文件详情

```bash
pan123 stat <文件名或ID...>

# 示例
pan123 stat file.txt
pan123 stat 123456
```

### refresh - 刷新缓存

```bash
pan123 refresh
```

清除当前目录缓存，重新加载文件列表。

### clear - 清屏

```bash
pan123 clear
```

清除终端屏幕（仅在 Shell 模式下）。

### shell - 交互式 Shell

```bash
pan123 shell
```

启动交互式命令行界面，支持：
- Tab 键自动补全命令
- 上下箭头浏览历史命令
- Ctrl+C 取消当前输入
- Ctrl+D 或 `exit` 退出

### help - 帮助

```bash
pan123 help
```

显示帮助信息。

## 配置

### Token 存储

Token 自动存储在系统密钥链中：
- **Linux**: Secret Service API (gnome-keyring, kwallet)
- **macOS**: Keychain
- **Windows**: Credential Manager

### 工作目录

当前工作目录信息存储在：
```
~/.config/pan123/cwd.json  (Linux/macOS)
%APPDATA%\pan123\cwd.json  (Windows)
```

### 下载断点续传

下载元数据存储在：
```
~/.config/pan123/resume/  (Linux/macOS)
%APPDATA%\pan123\resume\  (Windows)
```

## 高级用法

### 批量上传

```bash
# 上传多个文件
for file in *.txt; do
    pan123 upload "$file"
done

# 使用并发上传大目录
pan123 upload large-directory --jobs 8
```

### 批量下载

```bash
# 下载当前目录所有文件
pan123 ls | grep -v "^d" | awk '{print $NF}' | xargs pan123 download
```

### 脚本集成

```bash
#!/bin/bash

# 检查登录状态
if ! pan123 status | grep -q "Valid"; then
    echo "请先登录"
    pan123 login
fi

# 自动备份
pan123 cd Backup
pan123 upload /path/to/backup --duplicate overwrite
```

## 性能优化

### 上传优化

- 使用 `--jobs` 参数增加并发数（建议 4-8）
- 大文件自动分片上传（16MB/片）
- 支持秒传（文件 MD5 匹配）

### 下载优化

- 256KB 缓冲区
- 支持断点续传
- 自动重试失败的请求

### 限流控制

通过环境变量设置 API 请求限流：

```bash
export PAN123_RATE_LIMIT=10  # 每秒最多 10 个请求
pan123 upload large-directory
```

## 故障排除

### Token 失效

```bash
# 检查 Token 状态
pan123 status

# 重新登录
pan123 login
```

### 上传失败

```bash
# 增加重试次数
pan123 upload file.txt --retries 10

# 减少并发数
pan123 upload directory --jobs 2
```

### 下载中断

下载会自动保存进度，重新运行相同命令即可继续：

```bash
pan123 download large-file.zip
# 中断后重新运行
pan123 download large-file.zip  # 自动从断点继续
```

### 网络问题

```bash
# 设置更多重试次数
pan123 download file.txt --retries 10
```

## 开发

### 项目结构

```
pan123-rs/
├── crates/
│   ├── pan123-sdk/      # 核心 SDK 库
│   │   ├── src/
│   │   │   ├── client.rs       # API 客户端
│   │   │   ├── models.rs       # 数据模型
│   │   │   ├── transfer.rs     # 传输逻辑
│   │   │   ├── error.rs        # 错误处理
│   │   │   └── config.rs       # 配置管理
│   │   └── Cargo.toml
│   └── pan123-cli/      # 命令行工具
│       ├── src/
│       │   ├── main.rs         # 入口
│       │   ├── cli.rs          # CLI 逻辑
│       │   └── icons.rs        # 图标
│       └── Cargo.toml
├── scripts/
│   ├── build.sh         # Linux/macOS 构建脚本
│   └── build.ps1        # Windows 构建脚本
├── Cargo.toml           # Workspace 配置
└── README.md
```

### 构建

```bash
# 开发构建
cargo build

# 发布构建
cargo build --release

# 运行测试
cargo test

# 运行示例
cargo run --example complete_usage
```

### 作为库使用

在 `Cargo.toml` 中添加：

```toml
[dependencies]
pan123-sdk = { path = "crates/pan123-sdk" }
```

示例代码：

```rust
use pan123_sdk::{Pan123Client, DuplicateMode};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 创建客户端
    let mut client = Pan123Client::new(None)?;
    
    // 登录
    client.login_by_qrcode()?;
    
    // 列出文件
    let files = client.get_file_list(0, 1, 100)?;
    for file in files {
        println!("{}: {}", file.file_id, file.file_name);
    }
    
    // 上传文件
    let file_info = client.upload_file(
        "test.txt",
        0,
        DuplicateMode::KeepBoth
    )?;
    println!("上传成功: {}", file_info.file_name);
    
    Ok(())
}
```

## 许可证

MIT License

## 贡献

欢迎提交 Issue 和 Pull Request！

## 致谢

- [123pan](https://www.123pan.com/) - 提供云存储服务
- Rust 社区 - 提供优秀的生态系统

## 免责声明

本项目仅供学习交流使用，请勿用于商业用途。使用本工具产生的任何问题由使用者自行承担。
