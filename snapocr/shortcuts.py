"""注册 / 注销 COSMIC 全局快捷键。

Wayland 客户端拿不到全局热键，这是协议的固有设计，不是缺功能。COSMIC 的
出路是它自己的自定义快捷键配置：`Spawn("命令")` 动作。所以「设置快捷键」
在这边不是运行时能力，而是**安装步骤** —— 我们直接写它的配置文件。

配置格式取自官方定义（pop-os/cosmic-settings-daemon 的 config/src/shortcuts）：

    {
        (modifiers: [Ctrl, Alt], key: "a"): Spawn("/path/to/snapocr shot"),
    }

修饰键取值 Super / Alt / Ctrl / Shift；键名是 xkbcommon 的 keysym 名
（去掉 `KEY_` 前缀）。cosmic-comp 用 notify 监听这个文件，改完即时生效。
"""

from __future__ import annotations

import re
import shutil
from pathlib import Path

CONFIG = (
    Path.home()
    / ".config/cosmic/com.system76.CosmicSettings.Shortcuts/v1/custom"
)

# 沿用 macOS 版键位：⌃⌥A / ⌃⌥S（Option 在 Linux 上即 Alt）。
# 已核对 COSMIC 默认快捷键中 Ctrl+Alt 组合完全未被占用。
BINDINGS = [
    ("[Ctrl, Alt]", "a", "shot", "SnapOCR 截图"),
    ("[Ctrl, Alt]", "s", "ocr", "SnapOCR 取字"),
]

_MARK = "snapocr"


def _launcher() -> Path:
    path = Path(__file__).resolve().parent.parent / "bin" / "snapocr"
    if not path.is_file():
        raise FileNotFoundError(f"找不到启动器：{path}")
    return path


def _lines() -> list[str]:
    exe = _launcher()
    return [
        f'    (modifiers: {mods}, key: "{key}", description: "{desc}"):'
        f' Spawn("{exe} {sub}"),'
        for mods, key, sub, desc in BINDINGS
    ]


def _read_existing() -> list[str]:
    """读出用户已有的自定义快捷键行（剔除我们自己写过的）。"""
    if not CONFIG.is_file():
        return []
    text = CONFIG.read_text(encoding="utf-8").strip()
    if not text:
        return []
    # 去掉最外层的 { }，按行保留内容。RON 是个 map，逐行处理足够安全：
    # 我们只增删自己那两行，不去解析用户写的东西。
    inner = text
    if inner.startswith("{"):
        inner = inner[1:]
    if inner.endswith("}"):
        inner = inner[:-1]
    return [
        line for line in inner.splitlines()
        if line.strip() and _MARK not in line
    ]


def _write(lines: list[str]) -> None:
    CONFIG.parent.mkdir(parents=True, exist_ok=True)
    if CONFIG.is_file() and CONFIG.stat().st_size:
        # 动别人的桌面配置之前先留个后路。
        shutil.copy2(CONFIG, CONFIG.with_suffix(".snapocr-backup"))
    body = "\n".join(lines)
    CONFIG.write_text(f"{{\n{body}\n}}\n", encoding="utf-8")


def install() -> str:
    kept = _read_existing()
    _write(kept + _lines())
    exe = _launcher()
    detail = "\n".join(
        f"  Ctrl+Alt+{key.upper()}   {desc}" for _m, key, _s, desc in BINDINGS
    )
    return (
        f"已写入 {CONFIG}\n{detail}\n\n"
        f"启动器：{exe}\n"
        "cosmic-comp 监听该文件，通常即刻生效；若没反应，"
        "在「设置 → 键盘 → 快捷键」里看一眼即可。"
    )


def uninstall() -> str:
    kept = _read_existing()
    if not CONFIG.is_file():
        return "本来就没有配置文件，无需处理。"
    if kept:
        _write(kept)
        return f"已从 {CONFIG} 移除 SnapOCR 的快捷键，保留了其它 {len(kept)} 条。"
    CONFIG.unlink()
    return f"已删除 {CONFIG}（其中只有 SnapOCR 的快捷键）。"


def status() -> str:
    if not CONFIG.is_file():
        return "未注册（配置文件不存在）"
    text = CONFIG.read_text(encoding="utf-8")
    ours = len(re.findall(re.escape(_MARK), text))
    return f"{CONFIG}\n已注册 {ours} 条 SnapOCR 快捷键" if ours else "未注册"
