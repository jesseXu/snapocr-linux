"""识别结果窗口：可编辑文本框 + 复制。

设计要点是**文本可编辑** —— 这正是砍掉全屏就地选取后换来的好处：
识别错了当场改，不必重来一次。
"""

from __future__ import annotations

import threading

import gi

gi.require_version("Gtk", "4.0")
from gi.repository import Gio, GLib, Gtk  # noqa: E402

from . import clipboard  # noqa: E402

_APP_ID = "com.jessezoo.snapocr"


class _ResultWindow(Gtk.ApplicationWindow):
    def __init__(self, app: Gtk.Application, recognize):
        super().__init__(application=app, title="识别结果")
        self.set_default_size(760, 520)
        self._recognize = recognize

        header = Gtk.HeaderBar()
        self.set_titlebar(header)
        self._copy_button = Gtk.Button(label="全部复制")
        self._copy_button.add_css_class("suggested-action")
        self._copy_button.connect("clicked", self._on_copy)
        self._copy_button.set_sensitive(False)
        header.pack_end(self._copy_button)

        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.set_child(box)

        scroller = Gtk.ScrolledWindow(vexpand=True)
        self._view = Gtk.TextView(
            editable=True,
            wrap_mode=Gtk.WrapMode.WORD_CHAR,
            top_margin=12,
            bottom_margin=12,
            left_margin=12,
            right_margin=12,
        )
        self._buffer = self._view.get_buffer()
        self._buffer.set_text("识别中…")
        self._view.set_sensitive(False)
        scroller.set_child(self._view)
        box.append(scroller)

        self._status = Gtk.Label(xalign=0, margin_start=12, margin_end=12, margin_top=6, margin_bottom=6)
        self._status.add_css_class("dim-label")
        self._status.set_text("正在识别…")
        box.append(self._status)

        # Esc 关闭，和框选浮层保持一致的退出习惯。
        keys = Gtk.EventControllerKey()
        keys.connect("key-pressed", self._on_key)
        self.add_controller(keys)

        threading.Thread(target=self._worker, daemon=True).start()

    def _worker(self) -> None:
        try:
            text = self._recognize()
            GLib.idle_add(self._done, text, None)
        except Exception as exc:  # 识别失败也要让窗口可用，别静默卡在「识别中」
            GLib.idle_add(self._done, "", exc)

    def _done(self, text: str, error: Exception | None) -> bool:
        self._view.set_sensitive(True)
        if error is not None:
            self._buffer.set_text("")
            self._status.set_text(f"识别失败：{error}")
            return False
        if not text.strip():
            self._buffer.set_text("")
            self._status.set_text("没有识别到文字")
            return False

        self._buffer.set_text(text)
        self._copy_button.set_sensitive(True)
        # 顺手全量复制一份：多数时候用户就是想要全部文字，省掉一次点击。
        # 想要局部就在框里选了再按「全部复制」旁边的 Ctrl+C。
        try:
            clipboard.write_text(text)
            note = "已全部复制到剪贴板"
        except Exception:
            note = "自动复制失败，可点右上角按钮"
        chars = len(text.replace("\n", ""))
        self._status.set_text(f"{chars} 字 · {note} · 可直接编辑后再复制 · Esc 关闭")
        return False

    def _on_copy(self, _button: Gtk.Button) -> None:
        start, end = self._buffer.get_bounds()
        text = self._buffer.get_text(start, end, False)
        try:
            clipboard.write_text(text)
            self._status.set_text("已复制到剪贴板")
        except Exception as exc:
            self._status.set_text(f"复制失败：{exc}")

    def _on_key(self, _c, keyval, _code, _state) -> bool:
        if keyval == 0xFF1B:  # Escape
            self.close()
            return True
        return False


def show(recognize) -> int:
    """开一个窗口，在后台线程跑 `recognize()` 并把结果填进去。

    先出窗口再识别（而不是识别完再出窗口）：识别要几百毫秒到数秒，
    先给反馈才不会让人以为快捷键没生效。
    """
    # NON_UNIQUE 是必须的:默认的单实例语义下,第一个结果窗还开着时再次
    # 调用只会去激活旧窗口然后静默退出,新识别的文字根本不显示。
    # 本工具由快捷键反复触发,每次都该是独立的一次结果。
    app = Gtk.Application(
        application_id=_APP_ID, flags=Gio.ApplicationFlags.NON_UNIQUE
    )

    def on_activate(a: Gtk.Application) -> None:
        _ResultWindow(a, recognize).present()

    app.connect("activate", on_activate)
    return app.run([])
