"""依赖自检。

装漏一个包的表现往往是运行时一句看不懂的报错，不如开一个命令一次说清楚
缺什么、以及补齐它的确切命令。
"""

from __future__ import annotations

import shutil
import subprocess

from . import paths

# (检查名, 检测函数, 缺失时需要的 apt 包)
_APT_HINTS = {
    "wl-copy": "wl-clipboard",
    "tesseract": "tesseract-ocr",
    "chi_sim": "tesseract-ocr-chi-sim",
    "chi_tra": "tesseract-ocr-chi-tra",
    "eng": "tesseract-ocr-eng",
    "GTK4": "python3-gi gir1.2-gtk-4.0",
    "cairo": "python3-cairo",
}


def _tesseract_langs() -> set[str]:
    exe = shutil.which("tesseract")
    if not exe:
        return set()
    try:
        out = subprocess.run(
            [exe, "--list-langs"], capture_output=True, text=True, timeout=10
        )
        return {l.strip() for l in out.stdout.splitlines()[1:] if l.strip()}
    except (OSError, subprocess.SubprocessError):
        return set()


def check() -> tuple[list[tuple[str, bool, str]], list[str]]:
    """返回 (检查项, 缺失的 apt 包)。检查项是 (名字, 是否通过, 说明)。"""
    results: list[tuple[str, bool, str]] = []
    missing: list[str] = []

    def record(name: str, ok: bool, detail: str) -> None:
        results.append((name, ok, detail))
        if not ok and name in _APT_HINTS:
            for pkg in _APT_HINTS[name].split():
                if pkg not in missing:
                    missing.append(pkg)

    try:
        binary = paths.shot_binary()
        record("snapocr-shot", True, binary)
    except FileNotFoundError as exc:
        record("snapocr-shot", False, str(exc))

    wl = shutil.which("wl-copy")
    record("wl-copy", bool(wl), wl or "missing — clipboard unavailable")

    tess = shutil.which("tesseract")
    record("tesseract", bool(tess), tess or "missing — OCR unavailable")
    langs = _tesseract_langs()
    for lang, label in (("chi_sim", "Simplified Chinese"), ("chi_tra", "Traditional Chinese"), ("eng", "English")):
        record(lang, lang in langs, f"{label} language data" + ("" if lang in langs else " — missing"))

    try:
        import gi

        gi.require_version("Gtk", "4.0")
        from gi.repository import Gtk  # noqa: F401

        record("GTK4", True, "result window and markup editor available")
    except Exception as exc:
        record("GTK4", False, f"missing: {exc}")

    try:
        import cairo

        record("cairo", True, f"pycairo {cairo.version}")
    except Exception as exc:
        record("cairo", False, f"missing: {exc}")

    return results, missing


def report() -> str:
    results, missing = check()
    width = max(len(name) for name, _ok, _d in results)
    lines = [
        f"  {'OK  ' if ok else 'MISS'}  {name:<{width}}  {detail}"
        for name, ok, detail in results
    ]
    body = "\n".join(lines)
    if missing:
        body += "\n\nInstall what is missing:\n  sudo apt install " + " ".join(missing)
    else:
        body += "\n\nAll dependencies present."
    return body
