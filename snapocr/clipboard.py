"""剪贴板写入。"""

from __future__ import annotations

import shutil
import subprocess


class ClipboardUnavailable(Exception):
    pass


def _require(tool: str) -> str:
    path = shutil.which(tool)
    if not path:
        raise ClipboardUnavailable(
            f"缺少 {tool}，请安装：sudo apt install wl-clipboard"
        )
    return path


def write_image(png: bytes) -> None:
    """把 PNG 写进剪贴板。

    Wayland 的剪贴板需要一个存活的进程持有选区所有权，wl-copy 会自己
    fork 一个后台进程来伺候后续的粘贴请求，所以我们退出后内容依然在。
    """
    subprocess.run(
        [_require("wl-copy"), "--type", "image/png"],
        input=png,
        check=True,
        timeout=10,
    )


def write_text(text: str) -> None:
    subprocess.run(
        [_require("wl-copy"), "--type", "text/plain;charset=utf-8"],
        input=text.encode("utf-8"),
        check=True,
        timeout=10,
    )
