#!/usr/bin/env python3
"""为一众 AI 客户端配置 pan123 MCP server 与 Skill。

install.sh / install.bat 在装好二进制后调用本脚本，保证两边行为一致。

设计约束（均来自对各客户端实际配置的调研）：
  * 配置文件是 JSON 时必须「读-改-写」，绝不能整体重写——用户的
    ~/.claude.json 里有大量无关配置，破坏它等于清空用户的 Claude 状态。
  * Codex 用 TOML 且有官方 `codex mcp add`，优先调用它，避免手写格式出错。
  * Codex 没有 skill 机制，只配 MCP。
  * 已存在 pan123 条目时跳过并提示，不覆盖用户的自定义配置。

退出码：0 全部成功或跳过；2 至少一项失败（调用方据此提示）。
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys

SERVER_NAME = "pan123"
MCP_ENTRY = {"command": "pan123", "args": ["mcp"]}
SKILLS = ("pan123-mcp", "pan123-cli")


def _setup_console() -> None:
    """让中文与 ✓ 等符号在 GBK 控制台（中文版 Windows 默认）下也能输出。

    Windows 中文 locale 的默认代码页是 GBK，无法编码 `✓`，直接 print 会抛
    UnicodeEncodeError。这里优先切到 UTF-8；失败则退回替换非 GBK 字符，
    绝不让输出编码问题中断配置流程。
    """
    for stream in (sys.stdout, sys.stderr):
        reconfigure = getattr(stream, "reconfigure", None)
        if reconfigure is not None:
            try:
                reconfigure(encoding="utf-8", errors="replace")
                return
            except (ValueError, OSError):
                pass
    os.environ.setdefault("PYTHONIOENCODING", "utf-8")


# 名称 | 配置路径 | 是否支持 skill | skill 目录
CLIENTS = [
    ("Claude Code", ".claude.json", True, ".claude/skills"),
    ("Codex", ".codex/config.toml", False, None),
    ("CodeBuddy", ".codebuddy/mcp.json", True, ".codebuddy/skills"),
    ("WorkBuddy", ".workbuddy/mcp.json", True, ".workbuddy/skills"),
]

OK, SKIP, FAIL = 0, 1, 2


def _read_json(path: str):
    """读取 JSON 配置；不存在返回空字典。解析失败抛 ValueError。"""
    if not os.path.exists(path):
        return {}
    with open(path, "rb") as fh:
        raw = fh.read()
    if not raw.strip():
        return {}
    try:
        # utf-8-sig：容忍 Windows 编辑器留下的 BOM。
        data = json.loads(raw.decode("utf-8-sig"))
    except Exception as err:  # noqa: BLE001 - 面向用户需打印原始原因
        raise ValueError(str(err)) from err
    if not isinstance(data, dict):
        raise ValueError("顶层不是 JSON 对象")
    return data


def _write_json(path: str, data: dict) -> None:
    parent = os.path.dirname(path)
    if parent:
        os.makedirs(parent, exist_ok=True)
    # 先写临时文件再原子替换，避免中途失败留下半个损坏的配置文件。
    tmp = path + ".pan123.tmp"
    with open(tmp, "w", encoding="utf-8") as fh:
        json.dump(data, fh, ensure_ascii=False, indent=2)
    os.replace(tmp, path)


def configure_json_mcp(path: str) -> int:
    """把 MCP 条目合并进 JSON 配置，保留文件中已有的其它内容。"""
    try:
        data = _read_json(path)
    except ValueError as err:
        print(f"  ! JSON 解析失败，未改动此文件：{err}")
        return FAIL

    servers = data.get("mcpServers")
    if isinstance(servers, dict) and SERVER_NAME in servers:
        print("  · 已存在 pan123 条目，保留现有配置")
        return SKIP
    if not isinstance(servers, dict):
        servers = {}
        data["mcpServers"] = servers
    servers[SERVER_NAME] = dict(MCP_ENTRY)

    try:
        _write_json(path, data)
    except OSError as err:
        print(f"  ! 写入失败：{err}")
        return FAIL
    print(f"  ✓ 已写入 {path}")
    return OK


def configure_codex(path: str) -> int:
    """Codex 优先用其自带命令，失败再回退到直接追加 TOML 段落。

    注意：`codex mcp add` 只会写它自己认定的 CODEX_HOME（默认 ~/.codex），
    忽略我们传入的 --home。所以必须显式把 CODEX_HOME 指到目标 config.toml
    所在目录，否则测试用的临时 home 会漏写、真实配置反而被改动。
    """
    codex_home = os.path.dirname(path)
    env = {**os.environ, "CODEX_HOME": codex_home}

    codex = shutil.which("codex")
    if codex:
        try:
            listed = subprocess.run(
                [codex, "mcp", "list"],
                capture_output=True,
                text=True,
                timeout=30,
                env=env,
            )
            if SERVER_NAME in (listed.stdout or ""):
                print("  · 已存在 pan123 条目，保留现有配置")
                return SKIP
        except (OSError, subprocess.SubprocessError):
            pass  # 探测失败不代表不能写入，继续尝试注册

        try:
            added = subprocess.run(
                [codex, "mcp", "add", SERVER_NAME, "--", "pan123", "mcp"],
                capture_output=True,
                text=True,
                timeout=60,
                env=env,
            )
            if added.returncode == 0:
                print("  ✓ 已通过 codex mcp add 注册")
                return OK
            print("  · codex mcp add 失败，改为直接写入 config.toml")
        except (OSError, subprocess.SubprocessError) as err:
            print(f"  · codex mcp add 不可用（{err}），改为直接写入 config.toml")

    if os.path.exists(path):
        with open(path, "r", encoding="utf-8", errors="replace") as fh:
            if f"[mcp_servers.{SERVER_NAME}]" in fh.read():
                print("  · 已存在 pan123 条目，保留现有配置")
                return SKIP

    parent = os.path.dirname(path)
    if parent:
        os.makedirs(parent, exist_ok=True)
    try:
        with open(path, "a", encoding="utf-8") as fh:
            fh.write(
                f'\n[mcp_servers.{SERVER_NAME}]\n'
                'command = "pan123"\n'
                'args = ["mcp"]\n'
            )
    except OSError as err:
        print(f"  ! 写入失败：{err}")
        return FAIL
    print(f"  ✓ 已追加到 {path}")
    return OK


def install_skills(skill_src: str, dest_dir: str) -> int:
    if not os.path.isdir(skill_src):
        print(f"  ! 源目录不存在：{skill_src}")
        return FAIL
    try:
        os.makedirs(dest_dir, exist_ok=True)
        for skill in SKILLS:
            src = os.path.join(skill_src, skill)
            dst = os.path.join(dest_dir, skill)
            if os.path.isdir(src):
                shutil.copytree(src, dst, dirs_exist_ok=True)
    except OSError as err:
        print(f"  ! Skill 安装失败：{err}")
        return FAIL
    print(f"  ✓ Skill 已安装到 {dest_dir}")
    return OK


def main() -> int:
    parser = argparse.ArgumentParser(description="配置 pan123 到 AI 客户端")
    parser.add_argument(
        "--skill-src",
        help="仓库内 skills/ 目录路径；给定则安装 Skill",
    )
    parser.add_argument(
        "--home",
        default=os.path.expanduser("~"),
        help="用户主目录（默认当前用户 home）",
    )
    parser.add_argument(
        "--only",
        action="append",
        choices=[name for name, _, _, _ in CLIENTS],
        help="只配置指定客户端，可重复",
    )
    args = parser.parse_args()

    home = args.home
    failures = 0

    for name, rel_path, has_skill, skill_rel in CLIENTS:
        if args.only and name not in args.only:
            continue

        print(f"\n{name}")
        cfg_path = os.path.join(home, *rel_path.split("/"))

        if name == "Codex":
            rc = configure_codex(cfg_path)
        else:
            rc = configure_json_mcp(cfg_path)
        if rc == FAIL:
            failures += 1
            print(f"    可手动在 {cfg_path} 中加入 pan123 条目")

        if args.skill_src and has_skill:
            if install_skills(args.skill_src, os.path.join(home, *skill_rel.split("/"))) == FAIL:
                failures += 1

    print("\n后续步骤：")
    print("  1. 重启客户端以加载新配置")
    print("  2. 在你自己的终端执行 `pan123 login` 扫码登录（各客户端共享登录态）")
    print("  3. WorkBuddy 需到「自定义连接器 → MCP 服务管理」点「信任」才会生效")

    return 2 if failures else 0


if __name__ == "__main__":
    _setup_console()
    sys.exit(main())
