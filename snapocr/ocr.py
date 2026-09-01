"""文字识别：tesseract 子进程。

只需要「文本 + 阅读顺序」——因为 v1 砍掉了全屏就地选取，不再需要逐字
坐标，tesseract 就够用了。识别错了用户可以在文本框里直接改，这让准确率
从硬伤降级为可接受的差异。
"""

from __future__ import annotations

import shutil
import subprocess

# 简繁英一起上。多带一门语言的代价很小，漏识别的代价很大。
DEFAULT_LANGS = "chi_sim+chi_tra+eng"

_TIMEOUT_SECONDS = 60


class TesseractMissing(Exception):
    pass


def recognize(png: bytes, langs: str = DEFAULT_LANGS, dictionary: bool = True) -> str:
    """识别 PNG 里的文字。

    `dictionary=False` 关掉词典纠正，对应 macOS 版的「语言修正」开关：
    开着更适合中英文段落，关掉更忠实于代码与英数字（如 null 不会被
    纠成 nul1）。
    """
    exe = shutil.which("tesseract")
    if not exe:
        raise TesseractMissing(
            "缺少 tesseract，请安装："
            "sudo apt install tesseract-ocr tesseract-ocr-chi-sim tesseract-ocr-chi-tra"
        )

    cmd = [exe, "stdin", "stdout", "-l", langs]
    if not dictionary:
        cmd += ["-c", "load_system_dawg=0", "-c", "load_freq_dawg=0"]

    proc = subprocess.run(
        cmd, input=png, capture_output=True, timeout=_TIMEOUT_SECONDS
    )
    if proc.returncode != 0:
        stderr = proc.stderr.decode("utf-8", "replace").strip()
        raise RuntimeError(f"tesseract 失败：{stderr}")

    return _tidy(proc.stdout.decode("utf-8", "replace"))


def _tidy(text: str) -> str:
    """收尾清理：去掉行尾空白与首尾空行，保留段落结构。"""
    lines = [line.rstrip() for line in text.splitlines()]
    while lines and not lines[0].strip():
        lines.pop(0)
    while lines and not lines[-1].strip():
        lines.pop()
    return "\n".join(lines)
