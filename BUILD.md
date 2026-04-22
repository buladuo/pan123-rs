# 构建和发布指南

本文档说明如何构建和发布 pan123-rs 项目。

## 前置要求

- Rust 1.75 或更高版本
- Cargo
- Git

## 构建步骤

### Linux / macOS

```bash
# 1. 克隆项目
git clone <repository-url>
cd pan123-rs

# 2. 运行构建脚本
./scripts/build.sh

# 3. 构建产物位置
# - 二进制文件: target/release/pan123
# - 发布包: dist/pan123-<version>-<os>-<arch>.tar.gz
# - 解压后的目录: dist/pan123-<version>-<os>-<arch>/
```

构建脚本会自动：
1. 检查 Rust 环境
2. 清理旧的构建文件
3. 编译 Release 版本
4. 创建发布包（包含二进制文件、README、安装/卸载脚本）
5. 打包为 .tar.gz

### Windows

```powershell
# 1. 克隆项目
git clone <repository-url>
cd pan123-rs

# 2. 运行构建脚本
.\scripts\build.ps1

# 3. 构建产物位置
# - 二进制文件: target\release\pan123.exe
# - 发布包: dist\pan123-<version>-windows-<arch>.zip
# - 解压后的目录: dist\pan123-<version>-windows-<arch>\
```

构建脚本会自动：
1. 检查 Rust 环境
2. 清理旧的构建文件
3. 编译 Release 版本
4. 创建发布包（包含二进制文件、README、安装/卸载脚本）
5. 打包为 .zip

## 手动构建

如果不想使用构建脚本，可以手动构建：

```bash
# 开发构建
cargo build

# 发布构建
cargo build --release

# 运行测试
cargo test

# 检查代码
cargo clippy

# 格式化代码
cargo fmt
```

## 发布包内容

构建完成后，发布包包含以下文件：

### Linux / macOS (.tar.gz)

```
pan123-<version>-<os>-<arch>/
├── pan123              # 可执行文件
├── README.md           # 使用文档
├── LICENSE             # 许可证（如果存在）
├── install.sh          # 安装脚本
└── uninstall.sh        # 卸载脚本
```

### Windows (.zip)

```
pan123-<version>-windows-<arch>\
├── pan123.exe          # 可执行文件
├── README.md           # 使用文档
├── LICENSE             # 许可证（如果存在）
├── INSTALL.txt         # 安装说明
├── install.bat         # 安装脚本
└── uninstall.bat       # 卸载脚本
```

## 安装发布包

### Linux / macOS

```bash
# 解压
tar -xzf pan123-<version>-<os>-<arch>.tar.gz
cd pan123-<version>-<os>-<arch>

# 运行安装脚本
./install.sh

# 或手动安装
sudo cp pan123 /usr/local/bin/
sudo chmod +x /usr/local/bin/pan123
```

### Windows

```powershell
# 解压 ZIP 文件
# 右键点击 install.bat -> 以管理员身份运行

# 或手动安装
# 1. 复制 pan123.exe 到 C:\Program Files\pan123\
# 2. 将该目录添加到系统 PATH
```

## 验证安装

```bash
# 检查版本
pan123 --version

# 查看帮助
pan123 --help

# 测试登录
pan123 login
```

## 交叉编译

### 为其他平台编译

```bash
# 安装目标平台工具链
rustup target add x86_64-unknown-linux-gnu
rustup target add x86_64-pc-windows-gnu
rustup target add x86_64-apple-darwin
rustup target add aarch64-unknown-linux-gnu
rustup target add aarch64-apple-darwin

# 编译
cargo build --release --target x86_64-unknown-linux-gnu
cargo build --release --target x86_64-pc-windows-gnu
cargo build --release --target aarch64-apple-darwin
```

注意：交叉编译可能需要额外的链接器和系统库。

## 发布到 GitHub Releases

1. 创建 Git 标签：
```bash
git tag -a v0.1.0 -m "Release version 0.1.0"
git push origin v0.1.0
```

2. 在 GitHub 上创建 Release：
   - 上传构建好的发布包
   - 添加 Release Notes

3. 或使用 GitHub Actions 自动化发布（需要配置 CI/CD）

## 发布到 crates.io

```bash
# 登录 crates.io
cargo login

# 发布 SDK
cd crates/pan123-sdk
cargo publish

# 发布 CLI
cd ../pan123-cli
cargo publish
```

## 优化构建

### 减小二进制文件大小

在 `Cargo.toml` 中添加：

```toml
[profile.release]
opt-level = "z"     # 优化大小
lto = true          # 链接时优化
codegen-units = 1   # 更好的优化
strip = true        # 移除符号
panic = "abort"     # 减小 panic 处理代码
```

### 启用 CPU 特定优化

```bash
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

## 故障排除

### 构建失败

```bash
# 更新 Rust
rustup update

# 清理并重新构建
cargo clean
cargo build --release
```

### 依赖问题

```bash
# 更新依赖
cargo update

# 检查依赖树
cargo tree
```

### 链接错误

确保安装了必要的系统库：

**Linux (Ubuntu/Debian)**:
```bash
sudo apt-get install build-essential pkg-config libssl-dev
```

**macOS**:
```bash
xcode-select --install
```

## 持续集成

可以使用 GitHub Actions 自动化构建和发布流程。创建 `.github/workflows/release.yml`：

```yaml
name: Release

on:
  push:
    tags:
      - 'v*'

jobs:
  build:
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    
    steps:
      - uses: actions/checkout@v3
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      
      - name: Build
        run: cargo build --release
      
      - name: Upload artifacts
        uses: actions/upload-artifact@v3
        with:
          name: pan123-${{ matrix.os }}
          path: target/release/pan123*
```

## 更多信息

- [Rust 编译指南](https://doc.rust-lang.org/cargo/reference/build-scripts.html)
- [交叉编译指南](https://rust-lang.github.io/rustup/cross-compilation.html)
- [发布到 crates.io](https://doc.rust-lang.org/cargo/reference/publishing.html)
