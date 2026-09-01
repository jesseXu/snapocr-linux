"""SnapOCR for Linux —— 编排层。

    python3 -m snapocr shot    框选 → 图进剪贴板 → 通知（可保存）
    python3 -m snapocr ocr     框选 → 识别文字 → 文本窗口
"""

from __future__ import annotations

import pathlib
import struct
import sys

from . import clipboard, doctor, markup, notify, ocr, paths, shortcuts, shot, textwindow


USAGE = """用法：snapocr <命令>

  shot                     框选截图 → 图片进剪贴板
  ocr                      框选取字 → 可编辑的结果窗口
  markup [文件]            框选并标注（钢笔 / 箭头 / 数字标记点）
                           给文件路径则标注该图片
                           --clipboard 标注剪贴板里的图

  install [--shot 键] [--ocr 键] [--markup 键]
                           注册全局快捷键
                           默认 Ctrl+Alt+A / Ctrl+Alt+S / Ctrl+Alt+E
                           例：snapocr install --shot "Super+Shift+A"
                           装好后也可在「设置 → 键盘 → 键盘快捷键 →
                           自定义快捷键」里直接改键位
  uninstall                移除本工具注册的快捷键
  status                   查看快捷键注册情况
  doctor                   检查依赖是否齐全"""


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
        # 留足看清并点击的时间;错过也不要紧,还有 snapocr markup 这个入口。
        timeout_ms=6000,
    )
    if action == "save":
        target = paths.pictures_dir() / paths.screenshot_name()
        target.write_bytes(png)
        notify.show("已保存", str(target), timeout_ms=3000)
    elif action == "markup":
        return markup.show(png)
    return 0


def cmd_markup(args: list[str]) -> int:
    """截图并标注。

    标注编辑器原本只能从截图通知的按钮进入,通知几秒就消失、错过就没
    入口了。这里给它一个独立入口:直接框选进标注,或对已有图片/剪贴板
    里的图标注。
    """
    if args and args[0] == "--clipboard":
        import subprocess

        proc = subprocess.run(
            ["wl-paste", "--type", "image/png"], capture_output=True, timeout=10
        )
        if proc.returncode != 0 or not proc.stdout:
            print("剪贴板里没有图片", file=sys.stderr)
            return 1
        png = proc.stdout
    elif args:
        path = pathlib.Path(args[0]).expanduser()
        if not path.is_file():
            print(f"找不到文件：{path}", file=sys.stderr)
            return 1
        png = path.read_bytes()
    else:
        try:
            png = shot.select_region()
        except shot.Cancelled:
            return 0
    return markup.show(png)


def cmd_ocr() -> int:
    try:
        png = shot.select_region()
    except shot.Cancelled:
        return 0
    # 窗口先出来、识别在后台跑：识别要几百毫秒到数秒，
    # 先给反馈才不会让人以为快捷键没生效。
    return textwindow.show(lambda: ocr.recognize(png))


def cmd_install(args: list[str]) -> int:
    keys: dict[str, str] = {}
    i = 0
    while i < len(args):
        if args[i] in ("--shot", "--ocr", "--markup") and i + 1 < len(args):
            keys[args[i][2:]] = args[i + 1]
            i += 2
            continue
        print(f"无法识别的参数：{args[i]}", file=sys.stderr)
        return 64
    try:
        print(shortcuts.install(keys))
    except ValueError as exc:
        print(f"错误：{exc}", file=sys.stderr)
        return 64
    return 0


def cmd_uninstall() -> int:
    print(shortcuts.uninstall())
    return 0


def cmd_status() -> int:
    print(shortcuts.status())
    return 0


def cmd_doctor() -> int:
    results, missing = doctor.check()
    print(doctor.report())
    return 1 if missing or any(not ok for _n, ok, _d in results) else 0


def main(argv: list[str]) -> int:
    commands = {
        "shot": cmd_shot,
        "ocr": cmd_ocr,
        "markup": cmd_markup,
        "install": cmd_install,
        "uninstall": cmd_uninstall,
        "status": cmd_status,
        "doctor": cmd_doctor,
    }
    if len(argv) < 2 or argv[1] not in commands:
        print(USAGE, file=sys.stderr)
        return 64
    try:
        command = commands[argv[1]]
        return command(argv[2:]) if argv[1] in ("install", "markup") else command()
    except (clipboard.ClipboardUnavailable, ocr.TesseractMissing, FileNotFoundError) as exc:
        print(f"错误：{exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
