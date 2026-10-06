#!/usr/bin/env bash
# 将 pan123-cli skill 从本仓库安装到 agent 的个人 skill 目录。
#
# 用法:
#   ./scripts/install-skill.sh                 # 安装到默认目录
#   ./scripts/install-skill.sh --dir <路径>     # 安装到指定目录
#   ./scripts/install-skill.sh --uninstall      # 卸载
#
# 默认目标目录（可用环境变量 SKILLS_DIR 覆盖）:
#   $SKILLS_DIR，否则 ~/.agents/skills
#
# 也可复制到其它 agent 的 skills 目录，例如 ~/.cursor/skills。
set -euo pipefail

SKILL_NAME="pan123-cli"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC_DIR="$REPO_ROOT/skills/$SKILL_NAME"

TARGET_BASE="${SKILLS_DIR:-$HOME/.agents/skills}"
UNINSTALL=0

while [ $# -gt 0 ]; do
  case "$1" in
    --dir) TARGET_BASE="${2:?--dir 需要一个路径参数}"; shift 2 ;;
    --uninstall) UNINSTALL=1; shift ;;
    -h|--help)
      sed -n '2,12p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0 ;;
    *) echo "未知参数: $1" >&2; exit 2 ;;
  esac
done

DEST_DIR="$TARGET_BASE/$SKILL_NAME"

if [ "$UNINSTALL" -eq 1 ]; then
  if [ -d "$DEST_DIR" ]; then
    rm -rf -- "$DEST_DIR"
    echo "已卸载: $DEST_DIR"
  else
    echo "未安装: $DEST_DIR"
  fi
  exit 0
fi

if [ ! -f "$SRC_DIR/SKILL.md" ]; then
  echo "错误: 未找到源 skill: $SRC_DIR/SKILL.md" >&2
  exit 1
fi

mkdir -p -- "$DEST_DIR"
cp -f -- "$SRC_DIR/SKILL.md" "$DEST_DIR/SKILL.md"
for extra in "$SRC_DIR"/*.md; do
  base="$(basename "$extra")"
  [ "$base" = "SKILL.md" ] && continue
  cp -f -- "$extra" "$DEST_DIR/$base"
done

echo "已安装 skill '$SKILL_NAME' 到: $DEST_DIR"
echo "文件:"
ls -1 -- "$DEST_DIR"
echo
echo "提示: 新会话中该 skill 才会被加载；可用 --dir 指定其它 skills 目录（如 ~/.cursor/skills）。"
