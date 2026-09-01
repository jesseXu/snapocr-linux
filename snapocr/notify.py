"""桌面通知。

**只用来报错。** 正常流程的反馈走自绘 toast（见 shot.toast）——
通知规范里的动作按钮各家实现差异极大，cosmic-notifications 实测根本
不渲染按钮，dunst、mako 也不画，指望它反而最不通用。详见 DESIGN.md §8。

而报错恰恰适合通知：不需要按钮，会留在通知中心里等人看，
不像 toast 几秒就消失。由快捷键启动时 stderr 没人看得见，
静默失败比报错更糟。

`actions` 参数保留，但调用方现在都不传 —— 别指望它能显示出来。
"""

from __future__ import annotations

import gi

gi.require_version("Gtk", "4.0")
from gi.repository import Gio, GLib  # noqa: E402

_BUS_NAME = "org.freedesktop.Notifications"
_OBJ_PATH = "/org/freedesktop/Notifications"
_APP_NAME = "SnapOCR"


def show(
    summary: str,
    body: str = "",
    actions: list[tuple[str, str]] | None = None,
    timeout_ms: int = 4000,
    icon: str = "image-x-generic",
) -> str | None:
    """发一条通知。

    `actions` 是 (键, 显示文案) 列表。带动作时会阻塞等待用户点击，
    返回被点击的键；超时或用户关掉通知则返回 None。
    """
    actions = actions or []
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)

    flat: list[str] = []
    for key, label in actions:
        flat += [key, label]

    reply = bus.call_sync(
        _BUS_NAME,
        _OBJ_PATH,
        _BUS_NAME,
        "Notify",
        GLib.Variant(
            "(susssasa{sv}i)",
            (
                _APP_NAME,
                0,  # replaces_id
                icon,
                summary,
                body,
                flat,
                {"urgency": GLib.Variant("y", 1)},
                timeout_ms,
            ),
        ),
        GLib.VariantType("(u)"),
        Gio.DBusCallFlags.NONE,
        -1,
        None,
    )
    notification_id = reply.unpack()[0]

    if not actions:
        return None

    # 有动作按钮就得等：通知本身是异步的，进程退早了就收不到点击。
    loop = GLib.MainLoop()
    chosen: dict[str, str] = {}

    def on_action(_conn, _sender, _path, _iface, _signal, params):
        nid, key = params.unpack()
        if nid == notification_id:
            chosen["key"] = key
            loop.quit()

    def on_closed(_conn, _sender, _path, _iface, _signal, params):
        nid, _reason = params.unpack()
        if nid == notification_id:
            loop.quit()

    sub_action = bus.signal_subscribe(
        None, _BUS_NAME, "ActionInvoked", _OBJ_PATH, None,
        Gio.DBusSignalFlags.NONE, on_action,
    )
    sub_closed = bus.signal_subscribe(
        None, _BUS_NAME, "NotificationClosed", _OBJ_PATH, None,
        Gio.DBusSignalFlags.NONE, on_closed,
    )
    # 兜底：实测 cosmic-notifications 在通知超时后**不发** NotificationClosed，
    # 没有这个定时器进程就会永远挂在 loop.run() 上。
    GLib.timeout_add(timeout_ms + 2000, lambda: (loop.quit(), False)[1])

    try:
        loop.run()
    finally:
        bus.signal_unsubscribe(sub_action)
        bus.signal_unsubscribe(sub_closed)

    return chosen.get("key")
