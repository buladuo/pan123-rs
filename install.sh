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

# 查找二进制文件
BINARY_PATH=""
if [ -f "target/release/${BINARY_NAME}" ]; then
    BINARY_PATH="target/release/${BINARY_NAME}"
elif [ -f "${BINARY_NAME}" ]; then
    BINARY_PATH="${BINARY_NAME}"
else
    echo -e "${RED}错误: 未找到 ${BINARY_NAME}${NC}"
    echo ""
    echo "请先编译项目:"
    echo "  cargo build --release"
    echo ""
    echo "或运行构建脚本:"
    echo "  bash scripts/build.sh"
    echo ""
    exit 1
fi

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

# 检查权限并安装
if [ -w "$INSTALL_DIR" ]; then
    cp "${BINARY_PATH}" "${INSTALL_DIR}/${BINARY_NAME}"
    chmod +x "${INSTALL_DIR}/${BINARY_NAME}"
else
    echo "需要 sudo 权限来安装到 ${INSTALL_DIR}"
    sudo cp "${BINARY_PATH}" "${INSTALL_DIR}/${BINARY_NAME}"
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
