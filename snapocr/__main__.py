"""SnapOCR for Linux —— 编排层。

    python3 -m snapocr shot    框选 → 图进剪贴板 → 通知（可保存）
    python3 -m snapocr ocr     框选 → 识别文字 → 文本窗口
"""

from __future__ import annotations

import pathlib
import struct
import sys

from . import clipboard, doctor, markup, notify, ocr, paths, shortcuts, shot, textwindow


USAGE = """Usage: snapocr <command>

  shot                  select a region -> image goes to the clipboard
  ocr                   select a region -> text in an editable window
  markup [FILE]         select a region and annotate it
                        (pen / arrow / numbered markers)
                        FILE annotates that image instead
                        --clipboard annotates the image in the clipboard

  install [--shot KEY] [--ocr KEY] [--markup KEY]
                        register the global shortcuts
                        defaults: Ctrl+Alt+A / Ctrl+Alt+S / Ctrl+Alt+E
                        e.g. snapocr install --shot "Super+Shift+A"
                        you can also rebind them afterwards in
                        Settings -> Keyboard -> Keyboard Shortcuts ->
                        Custom Shortcuts
  uninstall             remove the shortcuts this tool registered
  status                show which shortcuts are registered
  doctor                check that all dependencies are present"""


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

    # 先进剪贴板再弹 toast —— 顺序不能反：截图的卖点就是「松手即可粘贴」，
    # 不能让它等一条提示走完。
    clipboard.write_image(png)

    width, height = png_size(png)
    action = shot.toast("copied", f"{width} x {height}")
    if action == "save":
        target = paths.pictures_dir() / paths.screenshot_name()
        target.write_bytes(png)
        shot.toast("saved", timeout_ms=1600)
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
            print("No image in the clipboard", file=sys.stderr)
            return 1
        png = proc.stdout
    elif args:
        path = pathlib.Path(args[0]).expanduser()
        if not path.is_file():
            print(f"File not found: {path}", file=sys.stderr)
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
        print(f"Unknown option: {args[i]}", file=sys.stderr)
        return 64
    try:
        print(shortcuts.install(keys))
    except ValueError as exc:
        print(f"Error: {exc}", file=sys.stderr)
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
    except Exception as exc:  # noqa: BLE001
        # 由快捷键启动时没人看得见 stderr,静默失败比报错更糟。
        # 通知在这里正合适:不需要按钮,而且会留在通知中心里等人看。
        print(f"Error: {exc}", file=sys.stderr)
        if argv[1] in ("shot", "ocr", "markup"):
            try:
                notify.show("SnapOCR error", str(exc), timeout_ms=8000)
            except Exception:  # noqa: BLE001
                pass
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
