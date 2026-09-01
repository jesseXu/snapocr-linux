"""保存路径与二进制定位。"""

from __future__ import annotations

import datetime as _dt
import os
import shutil
import subprocess
from pathlib import Path

_REPO_ROOT = Path(__file__).resolve().parent.parent


def shot_binary() -> str:
    """定位 snapocr-shot。

    优先用仓库内构建产物（开发时改完即用），其次找 PATH（安装后）。
    """
    for candidate in (
        _REPO_ROOT / "target" / "release" / "snapocr-shot",
        _REPO_ROOT / "target" / "debug" / "snapocr-shot",
    ):
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate)
    found = shutil.which("snapocr-shot")
    if found:
        return found
    raise FileNotFoundError(
        "snapocr-shot not found — build it: cargo build --release --manifest-path snapocr-shot/Cargo.toml"
    )


def pictures_dir() -> Path:
    """截图保存目录。

    跟随 XDG 用户目录而非硬编码「桌面」——Linux 上桌面目录未必存在
    （不少人关掉了桌面图标），而 XDG_PICTURES_DIR 是各发行版通用约定。
    """
    try:
        out = subprocess.run(
            ["xdg-user-dir", "PICTURES"], capture_output=True, text=True, timeout=2
        )
        path = Path(out.stdout.strip())
        if out.returncode == 0 and str(path) and path != Path.home():
            path.mkdir(parents=True, exist_ok=True)
            return path
    except (OSError, subprocess.SubprocessError):
        pass
    fallback = Path.home() / "Pictures"
    fallback.mkdir(parents=True, exist_ok=True)
    return fallback


def screenshot_name(now: _dt.datetime | None = None) -> str:
    """文件名不含空格，方便在 shell 里直接用。"""
    now = now or _dt.datetime.now()
    return f"Screenshot_{now:%Y-%m-%d_%H-%M-%S}.png"
