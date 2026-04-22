#!/bin/bash

set -e

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# 项目信息
PROJECT_NAME="pan123"
VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}  pan123-rs 构建脚本 (Linux/macOS)${NC}"
echo -e "${BLUE}  版本: ${VERSION}${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# 检查 Rust 环境
echo -e "${YELLOW}[1/5] 检查 Rust 环境...${NC}"
if ! command -v cargo &> /dev/null; then
    echo -e "${RED}错误: 未找到 cargo 命令${NC}"
    echo -e "${YELLOW}请先安装 Rust: https://rustup.rs/${NC}"
    exit 1
fi

RUST_VERSION=$(rustc --version)
echo -e "${GREEN}✓ Rust 环境正常: ${RUST_VERSION}${NC}"
echo ""

# 清理旧的构建
echo -e "${YELLOW}[2/5] 清理旧的构建文件...${NC}"
cargo clean
echo -e "${GREEN}✓ 清理完成${NC}"
echo ""

# 编译 Release 版本
echo -e "${YELLOW}[3/5] 编译 Release 版本...${NC}"
echo -e "${BLUE}这可能需要几分钟时间，请耐心等待...${NC}"
cargo build --release
echo -e "${GREEN}✓ 编译完成${NC}"
echo ""

# 检查编译产物
echo -e "${YELLOW}[4/5] 检查编译产物...${NC}"
BINARY_PATH="target/release/${PROJECT_NAME}"
if [ ! -f "$BINARY_PATH" ]; then
    echo -e "${RED}错误: 未找到编译产物 ${BINARY_PATH}${NC}"
    exit 1
fi

BINARY_SIZE=$(du -h "$BINARY_PATH" | cut -f1)
echo -e "${GREEN}✓ 二进制文件: ${BINARY_PATH} (${BINARY_SIZE})${NC}"
echo ""

# 创建发布包
echo -e "${YELLOW}[5/5] 创建发布包...${NC}"

# 检测操作系统和架构
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$ARCH" in
    x86_64)
        ARCH="x86_64"
        ;;
    aarch64|arm64)
        ARCH="aarch64"
        ;;
    *)
        echo -e "${YELLOW}警告: 未知架构 ${ARCH}，使用原始值${NC}"
        ;;
esac

RELEASE_NAME="${PROJECT_NAME}-${VERSION}-${OS}-${ARCH}"
RELEASE_DIR="dist/${RELEASE_NAME}"

# 创建发布目录
mkdir -p "$RELEASE_DIR"

# 复制文件
cp "$BINARY_PATH" "$RELEASE_DIR/${PROJECT_NAME}"
cp README.md "$RELEASE_DIR/" 2>/dev/null || echo "README.md not found, skipping"
cp LICENSE "$RELEASE_DIR/" 2>/dev/null || echo "LICENSE not found, skipping"

# 创建安装脚本
cat > "$RELEASE_DIR/install.sh" << 'EOF'
#!/bin/bash

set -e

BINARY_NAME="pan123"
INSTALL_DIR="/usr/local/bin"

echo "正在安装 ${BINARY_NAME}..."

# 检查权限
if [ ! -w "$INSTALL_DIR" ]; then
    echo "需要 sudo 权限来安装到 ${INSTALL_DIR}"
    sudo cp "${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
    sudo chmod +x "${INSTALL_DIR}/${BINARY_NAME}"
else
    cp "${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
    chmod +x "${INSTALL_DIR}/${BINARY_NAME}"
fi

echo "✓ 安装完成！"
echo ""
echo "运行 '${BINARY_NAME} --help' 查看帮助信息"
echo "运行 '${BINARY_NAME} login' 开始使用"
EOF

chmod +x "$RELEASE_DIR/install.sh"

# 创建卸载脚本
cat > "$RELEASE_DIR/uninstall.sh" << 'EOF'
#!/bin/bash

set -e

BINARY_NAME="pan123"
INSTALL_DIR="/usr/local/bin"

echo "正在卸载 ${BINARY_NAME}..."

if [ -f "${INSTALL_DIR}/${BINARY_NAME}" ]; then
    if [ ! -w "$INSTALL_DIR" ]; then
        echo "需要 sudo 权限来卸载"
        sudo rm "${INSTALL_DIR}/${BINARY_NAME}"
    else
        rm "${INSTALL_DIR}/${BINARY_NAME}"
    fi
    echo "✓ 卸载完成！"
else
    echo "未找到已安装的 ${BINARY_NAME}"
fi
EOF

chmod +x "$RELEASE_DIR/uninstall.sh"

# 打包
cd dist
tar -czf "${RELEASE_NAME}.tar.gz" "${RELEASE_NAME}"
cd ..

PACKAGE_SIZE=$(du -h "dist/${RELEASE_NAME}.tar.gz" | cut -f1)
echo -e "${GREEN}✓ 发布包已创建: dist/${RELEASE_NAME}.tar.gz (${PACKAGE_SIZE})${NC}"
echo ""

# 显示安装说明
echo -e "${BLUE}========================================${NC}"
echo -e "${GREEN}构建成功！${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""
echo -e "${YELLOW}安装方法 1 - 使用安装脚本:${NC}"
echo -e "  cd dist/${RELEASE_NAME}"
echo -e "  ./install.sh"
echo ""
echo -e "${YELLOW}安装方法 2 - 手动安装:${NC}"
echo -e "  sudo cp target/release/${PROJECT_NAME} /usr/local/bin/"
echo -e "  sudo chmod +x /usr/local/bin/${PROJECT_NAME}"
echo ""
echo -e "${YELLOW}安装方法 3 - 解压发布包:${NC}"
echo -e "  tar -xzf dist/${RELEASE_NAME}.tar.gz"
echo -e "  cd ${RELEASE_NAME}"
echo -e "  ./install.sh"
echo ""
echo -e "${YELLOW}验证安装:${NC}"
echo -e "  ${PROJECT_NAME} --version"
echo ""
echo -e "${YELLOW}开始使用:${NC}"
echo -e "  ${PROJECT_NAME} login"
echo -e "  ${PROJECT_NAME} shell"
echo ""
