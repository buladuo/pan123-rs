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
