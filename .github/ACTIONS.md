# GitHub Actions 使用指南

本项目配置了两个 GitHub Actions 工作流：

## 1. CI 工作流 (ci.yml)

### 触发条件
- 推送到 `main`、`master` 或 `develop` 分支
- 针对这些分支的 Pull Request

### 执行的检查
- **Check**: 检查代码是否能编译
- **Test**: 在 Linux、Windows、macOS 上运行测试
- **Format**: 检查代码格式（rustfmt）
- **Clippy**: 运行 Rust linter
- **Build**: 在三个平台上构建 Release 版本

### 本地运行相同检查

```bash
# 检查编译
cargo check --all-features

# 运行测试
cargo test --all-features

# 检查格式
cargo fmt --all -- --check

# 运行 clippy
cargo clippy --all-features -- -D warnings

# 构建
cargo build --release --all-features
```

## 2. Release 工作流 (release.yml)

### 触发条件
- 推送以 `v` 开头的 Git 标签（如 `v0.1.0`）
- 手动触发（workflow_dispatch）

### 构建的平台
- Linux x86_64
- Linux aarch64 (ARM64)
- macOS x86_64 (Intel)
- macOS aarch64 (Apple Silicon)
- Windows x86_64

### 发布内容
每个平台会生成一个压缩包，包含：
- 可执行文件
- README.md
- LICENSE
- QUICKSTART.md
- 安装脚本（install.sh 或 install.bat）

### 如何发布新版本

#### 方法 1: 使用 Git 标签（推荐）

```bash
# 1. 更新版本号
# 编辑 Cargo.toml，修改 version = "0.1.0" 为新版本

# 2. 提交更改
git add Cargo.toml
git commit -m "Bump version to 0.2.0"
git push

# 3. 创建并推送标签
git tag -a v0.2.0 -m "Release version 0.2.0"
git push origin v0.2.0

# 4. GitHub Actions 会自动开始构建和发布
```

#### 方法 2: 手动触发

1. 访问 GitHub 仓库的 Actions 页面
2. 选择 "Release" 工作流
3. 点击 "Run workflow"
4. 选择分支并运行

### 发布流程

1. **创建 Release**: 在 GitHub 上创建一个新的 Release
2. **并行构建**: 在 5 个平台上同时构建
3. **打包**: 创建 .tar.gz (Unix) 或 .zip (Windows) 压缩包
4. **上传**: 将所有压缩包上传到 Release
5. **生成校验和**: 创建 SHA256 校验和文件

### 构建时间

通常需要 10-20 分钟完成所有平台的构建。

### 查看构建状态

- 访问仓库的 Actions 页面
- 点击对应的工作流运行
- 查看每个 job 的日志

## 版本号规范

建议使用语义化版本（Semantic Versioning）：

- **主版本号**: 不兼容的 API 修改
- **次版本号**: 向下兼容的功能性新增
- **修订号**: 向下兼容的问题修正

示例：
- `v0.1.0` - 初始版本
- `v0.1.1` - Bug 修复
- `v0.2.0` - 新功能
- `v1.0.0` - 稳定版本

## 发布检查清单

在发布新版本前，确保：

- [ ] 更新 `Cargo.toml` 中的版本号
- [ ] 更新 `README.md` 中的版本相关信息
- [ ] 所有测试通过 (`cargo test`)
- [ ] 代码格式正确 (`cargo fmt`)
- [ ] Clippy 检查通过 (`cargo clippy`)
- [ ] 更新 CHANGELOG.md（如果有）
- [ ] 提交所有更改
- [ ] 创建并推送 Git 标签

## 故障排除

### 构建失败

1. 检查 Actions 日志中的错误信息
2. 在本地运行相同的构建命令
3. 确保所有依赖都在 `Cargo.toml` 中正确声明

### 交叉编译问题

Linux aarch64 构建需要交叉编译工具链，工作流已自动安装。如果失败：
- 检查是否需要额外的系统依赖
- 考虑使用 `cross` 工具进行交叉编译

### Release 创建失败

确保：
- 标签格式正确（以 `v` 开头）
- 标签尚未存在
- 有足够的权限创建 Release

## 自定义工作流

### 添加新的构建目标

编辑 `.github/workflows/release.yml`，在 `matrix.include` 中添加：

```yaml
- os: ubuntu-latest
  target: x86_64-unknown-linux-musl
  archive: tar.gz
```

### 修改触发条件

编辑工作流文件的 `on` 部分：

```yaml
on:
  push:
    tags:
      - 'v*'
    branches:
      - main
  pull_request:
```

### 添加额外的检查

在 `ci.yml` 中添加新的 job：

```yaml
security-audit:
  name: Security Audit
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - uses: actions-rs/audit-check@v1
      with:
        token: ${{ secrets.GITHUB_TOKEN }}
```

## 徽章

在 README.md 中添加状态徽章：

```markdown
![CI](https://github.com/yourusername/pan123-rs/workflows/CI/badge.svg)
![Release](https://github.com/yourusername/pan123-rs/workflows/Release/badge.svg)
```

## 更多资源

- [GitHub Actions 文档](https://docs.github.com/en/actions)
- [Rust CI 最佳实践](https://github.com/actions-rs)
- [语义化版本规范](https://semver.org/lang/zh-CN/)
