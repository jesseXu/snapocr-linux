"""SnapOCR for Linux —— 编排层。

    python3 -m snapocr shot    框选 → 图进剪贴板 → 通知（可保存）
    python3 -m snapocr ocr     框选 → 识别文字 → 文本窗口
"""

from __future__ import annotations

import struct
import sys

from . import clipboard, markup, notify, ocr, paths, shot, textwindow


def png_size(data: bytes) -> tuple[int, int]:
    """从 PNG 头部读尺寸。只为在通知里显示，不值得引入图像库。"""
    if len(data) < 24 or data[:8] != b"\x89PNG\r\n\x1a\n":
        return (0, 0)
    width, height = struct.unpack(">II", data[16:24])
    return (width, height)


def cmd_shot() -> int:
    try:
        png = shot.select_region()
    except shot.Cancelled:
        return 0

    clipboard.write_image(png)

    width, height = png_size(png)
    action = notify.show(
        "已复制到剪贴板",
        f"{width} × {height}",
        actions=[("save", "保存到图片"), ("markup", "标注")],
    )
    if action == "save":
        target = paths.pictures_dir() / paths.screenshot_name()
        target.write_bytes(png)
        notify.show("已保存", str(target), timeout_ms=3000)
    elif action == "markup":
        return markup.show(png)
    return 0


def cmd_ocr() -> int:
    try:
        png = shot.select_region()
    except shot.Cancelled:
        return 0
    # 窗口先出来、识别在后台跑：识别要几百毫秒到数秒，
    # 先给反馈才不会让人以为快捷键没生效。
    return textwindow.show(lambda: ocr.recognize(png))


def main(argv: list[str]) -> int:
    commands = {"shot": cmd_shot, "ocr": cmd_ocr}
    if len(argv) < 2 or argv[1] not in commands:
        print(f"用法：python3 -m snapocr {{{'|'.join(commands)}}}", file=sys.stderr)
        return 64
    try:
        return commands[argv[1]]()
    except (clipboard.ClipboardUnavailable, ocr.TesseractMissing, FileNotFoundError) as exc:
        print(f"错误：{exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
