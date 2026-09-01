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
DEFAULT_KEYS = {
    "shot": "Ctrl+Alt+A",
    "ocr": "Ctrl+Alt+S",
    "markup": "Ctrl+Alt+E",   # E = edit，沿用 macOS 版 toast 上的按键
}
_DESCRIPTIONS = {
    "shot": "SnapOCR Screenshot",
    "ocr": "SnapOCR Text Capture",
    "markup": "SnapOCR Screenshot & Markup",
}

_MARK = "snapocr"


def _launcher() -> Path:
    """定位启动器。

    快捷键配置里必须写绝对路径 —— cosmic-comp 启动命令时不带用户的 PATH。

    **先找源码目录、再找系统路径**，顺序不能反：源码目录里跑的时候要注册的
    显然是眼前这份代码，若优先返回 /usr/bin 就会把快捷键指到系统里那份可能
    已经过时的安装上，改了代码却怎么按都没变化。
    装成 .deb 之后本模块位于 dist-packages，其上层没有 bin/snapocr，
    自然会落到 /usr/bin —— 两种场景都对。
    """
    local = Path(__file__).resolve().parent.parent / "bin" / "snapocr"
    if local.is_file():
        return local
    packaged = Path("/usr/bin/snapocr")
    if packaged.is_file():
        return packaged
    found = shutil.which("snapocr")
    if found:
        return Path(found)
    raise FileNotFoundError("snapocr launcher not found")


# 用户写法 → COSMIC 的修饰键名
_MOD_ALIASES = {
    "ctrl": "Ctrl", "control": "Ctrl",
    "alt": "Alt", "option": "Alt", "opt": "Alt",
    "shift": "Shift",
    "super": "Super", "meta": "Super", "win": "Super", "cmd": "Super",
}


def parse_key(spec: str) -> tuple[str, str]:
    """把 `Ctrl+Alt+A` 解析成 (`[Ctrl, Alt]`, `a`)。

    键名用 xkbcommon 的 keysym 名（去掉 KEY_ 前缀）：单个字母小写，
    具名键保持原样（F1 / Escape / Print）。
    """
    parts = [p.strip() for p in spec.replace("-", "+").split("+") if p.strip()]
    if not parts:
        raise ValueError(f"cannot parse shortcut: {spec!r}")
    *mod_parts, key = parts

    mods: list[str] = []
    for m in mod_parts:
        canonical = _MOD_ALIASES.get(m.lower())
        if canonical is None:
            raise ValueError(
                f"unknown modifier {m!r} — use Ctrl / Alt / Shift / Super"
            )
        if canonical not in mods:
            mods.append(canonical)
    if not mods:
        raise ValueError(
            f"{spec!r} has no modifier — an unmodified global shortcut "
            "would swallow ordinary typing."
        )

    key = key.lower() if len(key) == 1 else key
    return f"[{', '.join(mods)}]", key


def _lines(keys: dict[str, str]) -> list[str]:
    exe = _launcher()
    out = []
    for sub, spec in keys.items():
        mods, key = parse_key(spec)
        out.append(
            f'    (modifiers: {mods}, key: "{key}", '
            f'description: "{_DESCRIPTIONS[sub]}"): Spawn("{exe} {sub}"),'
        )
    return out


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


def install(keys: dict[str, str] | None = None) -> str:
    keys = {**DEFAULT_KEYS, **(keys or {})}
    for spec in keys.values():   # 先全部解析通过再落盘，避免写出半截配置
        parse_key(spec)
    kept = _read_existing()
    _write(kept + _lines(keys))
    detail = "\n".join(
        f"  {keys[sub]:<16} {_DESCRIPTIONS[sub]}" for sub in keys
    )
    return (
        f"Wrote {CONFIG}\n{detail}\n\n"
        f"Launcher: {_launcher()}\n"
        "cosmic-comp watches this file, so it usually takes effect at once.\n"
        "To rebind later: Settings -> Keyboard -> Keyboard Shortcuts -> Custom Shortcuts."
    )


def uninstall() -> str:
    kept = _read_existing()
    if not CONFIG.is_file():
        return "Nothing to do — no config file."
    if kept:
        _write(kept)
        return f"Removed SnapOCR shortcuts from {CONFIG}; kept {len(kept)} other entries."
    CONFIG.unlink()
    return f"Deleted {CONFIG} (it held only SnapOCR shortcuts)."


def status() -> str:
    if not CONFIG.is_file():
        return "Not registered (no config file)"
    text = CONFIG.read_text(encoding="utf-8")
    ours = len(re.findall(re.escape(_MARK), text))
    return f"{CONFIG}\n{ours} SnapOCR shortcut(s) registered" if ours else "Not registered"
