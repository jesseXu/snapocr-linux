"""调用 snapocr-shot 完成「冻结抓屏 + 框选」，拿回 PNG 字节。"""

from __future__ import annotations

import subprocess

from . import paths

# 用户可能盯着屏幕想半天再框选，给足时间；snapocr-shot 自己也有看门狗。
_TIMEOUT_SECONDS = 180

# 与 snapocr-shot 的退出码约定
_EXIT_CANCELLED = 2
_EXIT_TOAST_SAVE = 10
_EXIT_TOAST_MARKUP = 11


class Cancelled(Exception):
    """用户按了 Esc，或没有框出有效选区。"""


def select_region() -> bytes:
    """显示框选浮层，返回选区的 PNG 字节。取消则抛 Cancelled。"""
    proc = subprocess.run(
        [paths.shot_binary(), "-"],
        capture_output=True,
        timeout=_TIMEOUT_SECONDS,
    )
    if proc.returncode == _EXIT_CANCELLED:
        raise Cancelled()
    if proc.returncode != 0:
        stderr = proc.stderr.decode("utf-8", "replace").strip()
        raise RuntimeError(f"抓屏失败（退出码 {proc.returncode}）：{stderr}")
    if not proc.stdout:
        raise RuntimeError("抓屏没有输出任何数据")
    return proc.stdout


def toast(title: str, body: str = "", hint: str = "", timeout_ms: int = 4000) -> str | None:
    """底部弹一条浮层，返回用户按了什么（"save" / "markup" / None）。

    自绘而非用桌面通知：通知规范里的动作按钮各家实现差异极大 ——
    cosmic-notifications 实测不渲染，dunst、mako 也都不画按钮。而本工具
    已经硬依赖 layer-shell，自绘的可移植性成本是零。详见 DESIGN.md §8。
    """
    cmd = [paths.shot_binary(), "--toast", "--title", title, "--timeout", str(timeout_ms)]
    if body:
        cmd += ["--body", body]
    if hint:
        cmd += ["--hint", hint]
    try:
        code = subprocess.run(cmd, timeout=timeout_ms / 1000 + 10).returncode
    except subprocess.SubprocessError:
        return None
    return {_EXIT_TOAST_SAVE: "save", _EXIT_TOAST_MARKUP: "markup"}.get(code)
