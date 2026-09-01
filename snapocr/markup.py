"""标注编辑器：钢笔 / 箭头 / 数字标记点。

macOS 版这部分是纯几何逻辑、与平台无关，这里是 GTK4 + Cairo 的直译。
唯一实质差异是坐标系：AppKit 原点在左下、y 向上，Cairo 原点在左上、
y 向下，所以这边不需要任何翻转。

关键约束（沿用 macOS 版）：**复制/保存时按原始像素重绘**，而不是把
屏幕上缩小显示的画布导出去。否则 4K 截图存下来会变成 1100 宽的糊图。
"""

from __future__ import annotations

import io
import math

import cairo
import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Gdk", "4.0")
from gi.repository import Gdk, Gio, GLib, Gtk  # noqa: E402

from . import clipboard, paths  # noqa: E402

_APP_ID = "com.jessezoo.snapocr.markup"

# 画布显示上限，不放大（小图保持原大小）。
_MAX_W, _MAX_H = 1100.0, 720.0
_LINE_WIDTH = 3.0
_MARKER_RADIUS = 11.0

PEN, ARROW, MARKER = "pen", "arrow", "marker"

# (名字, RGB)
_PALETTE = [
    ("red", (0.90, 0.22, 0.21)),
    ("yellow", (0.98, 0.75, 0.18)),
    ("green", (0.30, 0.69, 0.31)),
    ("blue", (0.13, 0.51, 0.90)),
    ("black", (0.0, 0.0, 0.0)),
    ("white", (1.0, 1.0, 1.0)),
]

_CSS = b"""
.swatch { min-width: 22px; min-height: 22px; padding: 0; border-radius: 4px; }
.swatch-red    { background-image: none; background-color: #e63935; }
.swatch-yellow { background-image: none; background-color: #fabf2e; }
.swatch-green  { background-image: none; background-color: #4db050; }
.swatch-blue   { background-image: none; background-color: #2182e6; }
.swatch-black  { background-image: none; background-color: #000000; }
.swatch-white  { background-image: none; background-color: #ffffff; }
"""


class _Annotation:
    __slots__ = ("tool", "points", "color")

    def __init__(self, tool: str, points: list[tuple[float, float]], color):
        self.tool = tool
        self.points = points
        self.color = color


def _draw_all(ctx: cairo.Context, annotations: list[_Annotation], scale: float) -> None:
    """画出全部标注。标记点按插入顺序自动编号 1..N。"""
    marker_number = 0
    for ann in annotations:
        if ann.tool == MARKER:
            marker_number += 1
            _draw_marker(ctx, ann, marker_number, scale)
        else:
            _draw_stroke(ctx, ann, scale)


def _draw_stroke(ctx: cairo.Context, ann: _Annotation, scale: float) -> None:
    if not ann.points:
        return
    ctx.save()
    ctx.set_source_rgb(*ann.color)
    ctx.set_line_width(_LINE_WIDTH * scale)
    ctx.set_line_cap(cairo.LINE_CAP_ROUND)
    ctx.set_line_join(cairo.LINE_JOIN_ROUND)

    first = ann.points[0]
    ctx.move_to(first[0] * scale, first[1] * scale)
    for x, y in ann.points[1:]:
        ctx.line_to(x * scale, y * scale)

    if ann.tool == ARROW and len(ann.points) >= 2:
        sx, sy = ann.points[0]
        ex, ey = ann.points[-1]
        sx, sy, ex, ey = sx * scale, sy * scale, ex * scale, ey * scale
        angle = math.atan2(ey - sy, ex - sx)
        length = max(12.0, _LINE_WIDTH * 4) * scale
        wing = math.pi / 7
        for sign in (-1, 1):
            ctx.move_to(ex, ey)
            ctx.line_to(
                ex + math.cos(angle + math.pi + sign * wing) * length,
                ey + math.sin(angle + math.pi + sign * wing) * length,
            )
    ctx.stroke()
    ctx.restore()


def _draw_marker(ctx: cairo.Context, ann: _Annotation, number: int, scale: float) -> None:
    """红底白字的圆形标记点，编号自动递增。单击放置，不需要拖。"""
    if not ann.points:
        return
    cx, cy = ann.points[0]
    cx, cy = cx * scale, cy * scale
    r = _MARKER_RADIUS * scale

    ctx.save()
    ctx.set_source_rgb(0.90, 0.22, 0.21)
    ctx.arc(cx, cy, r, 0, 2 * math.pi)
    ctx.fill()

    ctx.set_source_rgb(1, 1, 1)
    ctx.set_line_width(max(1.0, 1.5 * scale))
    ctx.arc(cx, cy, r - 0.75 * scale, 0, 2 * math.pi)
    ctx.stroke()

    text = str(number)
    ctx.select_font_face("sans-serif", cairo.FONT_SLANT_NORMAL, cairo.FONT_WEIGHT_BOLD)
    ctx.set_font_size(r * 1.25)
    extents = ctx.text_extents(text)
    ctx.move_to(cx - extents.width / 2 - extents.x_bearing,
                cy - extents.height / 2 - extents.y_bearing)
    ctx.show_text(text)
    ctx.restore()


# 工具栏图标用中性灰：自绘纹理不像 symbolic 图标那样会跟随主题重新着色，
# 这个灰度在浅色和深色主题下都读得出来。
_ICON_RGB = (0.54, 0.54, 0.56)
_ICON_PX = 32


def _tool_icon(tool: str) -> Gdk.Texture:
    """用编辑器自己的绘制函数画出该工具的图标。

    不用图标主题：一来「钢笔 / 箭头 / 编号标记点」没有公认的图标名，
    二来主题里缺了对应图标时按钮会变成空白，用户无从下手。
    自己画还有个好处 —— 图标就是这支工具的真实笔迹，不会画错。
    """
    surface = cairo.ImageSurface(cairo.FORMAT_ARGB32, _ICON_PX, _ICON_PX)
    ctx = cairo.Context(surface)
    if tool == PEN:
        points = [(5, 21), (10, 11), (16, 21), (22, 11), (27, 17)]
        _draw_stroke(ctx, _Annotation(PEN, points, _ICON_RGB), 1.0)
    elif tool == ARROW:
        _draw_stroke(ctx, _Annotation(ARROW, [(6, 25), (26, 7)], _ICON_RGB), 1.0)
    else:
        _draw_marker(ctx, _Annotation(MARKER, [(16, 16)], _ICON_RGB), 1, 1.0)
    buf = io.BytesIO()
    surface.write_to_png(buf)
    return Gdk.Texture.new_from_bytes(GLib.Bytes.new(buf.getvalue()))


class _MarkupWindow(Gtk.ApplicationWindow):
    def __init__(self, app: Gtk.Application, png: bytes):
        super().__init__(application=app, title="Markup")

        self._source = cairo.ImageSurface.create_from_png(io.BytesIO(png))
        self._orig_w = self._source.get_width()
        self._orig_h = self._source.get_height()
        # 只缩小不放大：小图按原大小显示才不糊。
        fit = min(_MAX_W / self._orig_w, _MAX_H / self._orig_h, 1.0)
        self._disp_w = round(self._orig_w * fit)
        self._disp_h = round(self._orig_h * fit)

        self._annotations: list[_Annotation] = []
        self._draft: _Annotation | None = None
        self._tool = PEN
        self._color = _PALETTE[0][1]

        provider = Gtk.CssProvider()
        provider.load_from_data(_CSS)
        Gtk.StyleContext.add_provider_for_display(
            Gdk.Display.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION
        )

        self.set_titlebar(self._build_header())
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        box.append(self._build_toolbar())

        self._area = Gtk.DrawingArea()
        self._area.set_content_width(self._disp_w)
        self._area.set_content_height(self._disp_h)
        self._area.set_draw_func(self._on_draw)
        box.append(self._area)
        self.set_child(box)

        drag = Gtk.GestureDrag()
        drag.connect("drag-begin", self._on_drag_begin)
        drag.connect("drag-update", self._on_drag_update)
        drag.connect("drag-end", self._on_drag_end)
        self._area.add_controller(drag)

        keys = Gtk.EventControllerKey()
        keys.connect("key-pressed", self._on_key)
        self.add_controller(keys)

    # ---------- 界面 ----------

    def _build_header(self) -> Gtk.HeaderBar:
        header = Gtk.HeaderBar()
        undo = Gtk.Button(icon_name="edit-undo-symbolic", tooltip_text="Undo (Ctrl+Z)")
        undo.connect("clicked", lambda _b: self._undo())
        header.pack_start(undo)

        save = Gtk.Button(label="Save")
        save.connect("clicked", lambda _b: self._save())
        header.pack_end(save)
        copy = Gtk.Button(label="Copy")
        copy.add_css_class("suggested-action")
        copy.connect("clicked", lambda _b: self._copy())
        header.pack_end(copy)
        return header

    def _build_toolbar(self) -> Gtk.Box:
        bar = Gtk.Box(
            orientation=Gtk.Orientation.HORIZONTAL, spacing=6,
            margin_start=8, margin_end=8, margin_top=6, margin_bottom=6,
        )
        tools = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL)
        tools.add_css_class("linked")
        first: Gtk.ToggleButton | None = None
        for tool, tip in (
            (PEN, "Pen — press and drag to draw freehand"),
            (ARROW, "Arrow — drag from start to end"),
            (MARKER, "Marker — click to place, numbered automatically"),
        ):
            image = Gtk.Image.new_from_paintable(_tool_icon(tool))
            image.set_pixel_size(20)
            btn = Gtk.ToggleButton(tooltip_text=tip)
            btn.set_child(image)
            if first is None:
                first = btn
                btn.set_active(True)
            else:
                btn.set_group(first)
            btn.connect("toggled", self._on_tool, tool)
            tools.append(btn)
        bar.append(tools)

        bar.append(Gtk.Separator(orientation=Gtk.Orientation.VERTICAL))

        colors = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=4)
        swatch_group: Gtk.ToggleButton | None = None
        for name, rgb in _PALETTE:
            btn = Gtk.ToggleButton()
            btn.add_css_class("swatch")
            btn.add_css_class(f"swatch-{name}")
            if swatch_group is None:
                swatch_group = btn
                btn.set_active(True)
            else:
                btn.set_group(swatch_group)
            btn.connect("toggled", self._on_color, rgb)
            colors.append(btn)
        bar.append(colors)
        return bar

    # ---------- 绘制 ----------

    def _on_draw(self, _area, ctx: cairo.Context, _w: int, _h: int) -> None:
        scale = self._disp_w / self._orig_w
        ctx.save()
        ctx.scale(scale, scale)
        ctx.set_source_surface(self._source, 0, 0)
        ctx.paint()
        ctx.restore()

        _draw_all(ctx, self._annotations, 1.0)
        if self._draft is not None:
            _draw_stroke(ctx, self._draft, 1.0)

    def _render_native(self) -> bytes:
        """按**原始像素**重绘并编码 PNG。

        画布是缩小显示的，直接导出画布会得到糊图；这里重新按原图尺寸
        画一遍，标注坐标同比放大。
        """
        surface = cairo.ImageSurface(cairo.FORMAT_ARGB32, self._orig_w, self._orig_h)
        ctx = cairo.Context(surface)
        ctx.set_source_surface(self._source, 0, 0)
        ctx.paint()
        _draw_all(ctx, self._annotations, self._orig_w / self._disp_w)
        buf = io.BytesIO()
        surface.write_to_png(buf)
        return buf.getvalue()

    # ---------- 交互 ----------

    def _on_tool(self, button: Gtk.ToggleButton, tool: str) -> None:
        if button.get_active():
            self._tool = tool

    def _on_color(self, button: Gtk.ToggleButton, rgb) -> None:
        if button.get_active():
            self._color = rgb

    def _on_drag_begin(self, gesture: Gtk.GestureDrag, x: float, y: float) -> None:
        if self._tool == MARKER:
            # 标记点是单击放置的，没有拖拽草稿。
            self._annotations.append(_Annotation(MARKER, [(x, y)], self._color))
            self._area.queue_draw()
            return
        self._draft = _Annotation(self._tool, [(x, y)], self._color)

    def _on_drag_update(self, gesture: Gtk.GestureDrag, dx: float, dy: float) -> None:
        if self._draft is None:
            return
        ok, sx, sy = gesture.get_start_point()
        if not ok:
            return
        point = (sx + dx, sy + dy)
        if self._draft.tool == PEN:
            self._draft.points.append(point)          # 累积手绘轨迹
        else:
            self._draft.points = [self._draft.points[0], point]  # 直线：只留首尾
        self._area.queue_draw()

    def _on_drag_end(self, gesture: Gtk.GestureDrag, dx: float, dy: float) -> None:
        draft, self._draft = self._draft, None
        if draft is None:
            return
        # 忽略没真正画出东西的误点
        if draft.tool == PEN and len(draft.points) < 2:
            self._area.queue_draw()
            return
        if draft.tool == ARROW:
            (ax, ay), (bx, by) = draft.points[0], draft.points[-1]
            if math.hypot(bx - ax, by - ay) < 3:
                self._area.queue_draw()
                return
        self._annotations.append(draft)
        self._area.queue_draw()

    def _undo(self) -> None:
        if self._annotations:
            self._annotations.pop()
            self._area.queue_draw()

    def _copy(self) -> None:
        clipboard.write_image(self._render_native())
        self.close()

    def _save(self) -> None:
        target = paths.pictures_dir() / paths.screenshot_name()
        target.write_bytes(self._render_native())
        self.close()

    def _on_key(self, _c, keyval: int, _code: int, state: Gdk.ModifierType) -> bool:
        ctrl = bool(state & Gdk.ModifierType.CONTROL_MASK)
        if keyval == Gdk.KEY_Escape:
            self.close()
            return True
        if ctrl and keyval in (Gdk.KEY_z, Gdk.KEY_Z):
            self._undo()
            return True
        if ctrl and keyval in (Gdk.KEY_c, Gdk.KEY_C):
            self._copy()
            return True
        if ctrl and keyval in (Gdk.KEY_s, Gdk.KEY_S):
            self._save()
            return True
        return False


def show(png: bytes) -> int:
    """打开标注编辑器。NON_UNIQUE 的理由同 textwindow。"""
    app = Gtk.Application(
        application_id=_APP_ID, flags=Gio.ApplicationFlags.NON_UNIQUE
    )
    app.connect("activate", lambda a: _MarkupWindow(a, png).present())
    return app.run([])
