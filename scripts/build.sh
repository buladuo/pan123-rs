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

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

BINARY_NAME="pan123"
INSTALL_DIR="/usr/local/bin"

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}  pan123-rs 安装程序${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# 检查是否已安装
if [ -f "${INSTALL_DIR}/${BINARY_NAME}" ]; then
    echo -e "${YELLOW}检测到已安装的版本${NC}"

    # 获取当前版本
    if CURRENT_VERSION=$(${INSTALL_DIR}/${BINARY_NAME} --version 2>/dev/null); then
        echo -e "当前版本: ${CURRENT_VERSION}"
    fi

    echo ""
    echo -e "${YELLOW}将卸载旧版本并安装新版本${NC}"
    echo ""

    # 检查是否有进程在运行
    if pgrep -x "${BINARY_NAME}" > /dev/null; then
        echo -e "${YELLOW}检测到 ${BINARY_NAME} 正在运行${NC}"
        read -p "是否终止运行中的进程? (y/N): " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            pkill -x "${BINARY_NAME}" || true
            sleep 1
        else
            echo -e "${RED}请先手动关闭 ${BINARY_NAME} 进程后重试${NC}"
            exit 1
        fi
    fi

    # 删除旧版本
    echo "正在删除旧版本..."
    if [ -w "$INSTALL_DIR" ]; then
        rm -f "${INSTALL_DIR}/${BINARY_NAME}"
    else
        sudo rm -f "${INSTALL_DIR}/${BINARY_NAME}"
    fi
    echo -e "${GREEN}✓ 旧版本已删除${NC}"
    echo ""
fi

# 安装新版本
echo "正在安装 ${BINARY_NAME}..."

if [ ! -f "${BINARY_NAME}" ]; then
    echo -e "${RED}错误: 未找到 ${BINARY_NAME} 文件${NC}"
    echo "请确保在解压后的目录中运行此脚本"
    exit 1
fi

# 检查权限并安装
if [ -w "$INSTALL_DIR" ]; then
    cp "${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
    chmod +x "${INSTALL_DIR}/${BINARY_NAME}"
else
    echo "需要 sudo 权限来安装到 ${INSTALL_DIR}"
    sudo cp "${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
    sudo chmod +x "${INSTALL_DIR}/${BINARY_NAME}"
fi

# 验证安装
if [ ! -f "${INSTALL_DIR}/${BINARY_NAME}" ]; then
    echo -e "${RED}错误: 安装验证失败${NC}"
    exit 1
fi

# 获取新版本
NEW_VERSION=$(${INSTALL_DIR}/${BINARY_NAME} --version 2>/dev/null || echo "未知版本")

echo -e "${GREEN}✓ 文件已复制到 ${INSTALL_DIR}${NC}"
echo ""
echo -e "${BLUE}========================================${NC}"
echo -e "${GREEN}✓ 安装完成！${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""
echo -e "安装版本: ${NEW_VERSION}"
echo -e "安装位置: ${INSTALL_DIR}/${BINARY_NAME}"
echo ""
echo -e "${YELLOW}快速开始:${NC}"
echo -e "  ${BINARY_NAME} --version    查看版本"
echo -e "  ${BINARY_NAME} --help       查看帮助信息"
echo -e "  ${BINARY_NAME} login        登录账号"
echo -e "  ${BINARY_NAME} shell        启动交互式 Shell"
echo ""
echo -e "${YELLOW}卸载方法:${NC}"
echo -e "  运行 ./uninstall.sh 或 sudo rm ${INSTALL_DIR}/${BINARY_NAME}"
echo ""
EOF

chmod +x "$RELEASE_DIR/install.sh"

# 创建卸载脚本
cat > "$RELEASE_DIR/uninstall.sh" << 'EOF'
#!/bin/bash

set -e

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

BINARY_NAME="pan123"
INSTALL_DIR="/usr/local/bin"

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}  pan123-rs 卸载程序${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# 检查是否已安装
if [ ! -f "${INSTALL_DIR}/${BINARY_NAME}" ]; then
    echo -e "${YELLOW}未找到已安装的 ${BINARY_NAME}${NC}"
    echo -e "安装位置: ${INSTALL_DIR}/${BINARY_NAME}"
    exit 0
fi

# 获取当前版本
if CURRENT_VERSION=$(${INSTALL_DIR}/${BINARY_NAME} --version 2>/dev/null); then
    echo -e "检测到安装版本: ${CURRENT_VERSION}"
else
    echo -e "检测到已安装的 ${BINARY_NAME}"
fi
echo -e "安装位置: ${INSTALL_DIR}/${BINARY_NAME}"
echo ""

# 确认卸载
read -p "确认卸载? (y/N): " -n 1 -r
echo
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo "已取消卸载"
    exit 0
fi

echo ""
echo "正在卸载..."

# 检查是否有进程在运行
if pgrep -x "${BINARY_NAME}" > /dev/null; then
    echo -e "${YELLOW}检测到 ${BINARY_NAME} 正在运行，尝试结束进程...${NC}"
    pkill -x "${BINARY_NAME}" || true
    sleep 1
fi

# 删除文件
if [ -w "$INSTALL_DIR" ]; then
    rm -f "${INSTALL_DIR}/${BINARY_NAME}"
else
    echo "需要 sudo 权限来卸载"
    sudo rm -f "${INSTALL_DIR}/${BINARY_NAME}"
fi

# 验证删除
if [ -f "${INSTALL_DIR}/${BINARY_NAME}" ]; then
    echo -e "${RED}错误: 删除失败${NC}"
    exit 1
fi

echo -e "${GREEN}✓ 已删除程序文件${NC}"
echo ""
echo -e "${BLUE}========================================${NC}"
echo -e "${GREEN}✓ 卸载完成！${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""
echo -e "${YELLOW}注意:${NC}"
echo -e "1. 配置文件保留在: ~/.config/pan123/ 或 ~/.pan123/"
echo -e "2. 如需完全清理，请手动删除配置目录"
echo ""
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
