//! 框选浮层：zwlr_layer_shell_v1。
//!
//! 每块屏幕一个 overlay 层的 layer surface，显示该屏的冻结画面并整体压暗；
//! 拖拽出的选区还原为全亮，松手即返回选区。
//!
//! 坐标有两套：
//!   - **逻辑坐标**：合成器给的 configure 尺寸与指针坐标，单位是「点」。
//!   - **物理像素**：抓屏得到的图，也是最终裁剪输出的单位。
//! 二者相差一个 buffer scale。所有对外返回的选区一律用物理像素。

use anyhow::{Context, Result};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1,
};
use wayland_client::{
    globals::GlobalList,
    Dispatch,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
    Connection, QueueHandle,
};

use crate::capture::Shot;

/// 选区，单位为物理像素，坐标原点在对应屏幕的左上角。
#[derive(Debug, Clone, Copy)]
pub struct Selection {
    pub output_index: usize,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// 压暗程度：选区外保留的亮度。与 macOS 版的 0.45 黑色蒙版等效。
const DIM_KEEP: u32 = 140; // 亮度 * 140/255 ≈ 0.55
/// 选区边框粗细（物理像素）。
const BORDER: i64 = 2;
/// 边框颜色（B, G, R）——COSMIC 强调色近似的蓝。
const BORDER_BGR: [u8; 3] = [0xE0, 0x90, 0x30];
/// 两帧之间的最小间隔（约 120fps）。
const MIN_FRAME_INTERVAL: std::time::Duration = std::time::Duration::from_millis(8);
/// 小于这个尺寸（物理像素）视为误点，不算选区。
const MIN_SELECTION: u32 = 3;

struct Overlay {
    layer: LayerSurface,
    shot: Shot,
    /// 渲染就绪的全亮像素，BGRX，尺寸同 shot。
    bright: Vec<u8>,
    /// 渲染就绪的压暗像素，BGRX，尺寸同 shot。
    dim: Vec<u8>,
    pool: Option<SlotPool>,
    /// 上次提交时间，用于节流。
    last_draw: Option<std::time::Instant>,
    /// 屏幕名（HDMI-A-1 等）。output 的枚举顺序在不同运行间并不稳定，
    /// 用下标标识屏幕会误导人，日志一律打名字。
    name: String,
    /// configure 给的逻辑尺寸。
    logical: (u32, u32),
    configured: bool,
}

impl Overlay {
    /// 逻辑坐标 → 物理像素的换算系数。
    fn scale(&self) -> f64 {
        if self.logical.0 == 0 {
            1.0
        } else {
            self.shot.width as f64 / self.logical.0 as f64
        }
    }
}

pub struct App {
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    shm: Shm,
    compositor: CompositorState,
    layer_shell: LayerShell,
    /// wp_cursor_shape：直接向合成器要一个十字光标，
    /// 免去加载光标主题、找图片、管理 surface 那一整套。
    cursor_shape: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,
    cursor_device: Option<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1>,

    overlays: Vec<Overlay>,
    /// 各屏冻结像素的副本。浮层在退出时就拆掉了，但裁剪要用到，所以单独留一份。
    shots: Vec<Shot>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,

    /// 指针当前所在的 overlay 下标。
    active: Option<usize>,
    /// 按下点与当前点，逻辑坐标，属于 `active` 那块屏。
    press: Option<(f64, f64)>,
    current: Option<(f64, f64)>,

    pub result: Option<Selection>,
    pub exit: bool,
}

impl App {
    pub fn new(globals: &GlobalList, qh: &QueueHandle<Self>) -> Result<Self> {
        Ok(App {
            compositor: CompositorState::bind(globals, qh).context("合成器缺少 wl_compositor")?,
            layer_shell: LayerShell::bind(globals, qh)
                .context("合成器缺少 zwlr_layer_shell_v1")?,
            shm: Shm::bind(globals, qh).context("合成器缺少 wl_shm")?,
            // 可选：没有就退回合成器默认光标，不影响主流程。
            cursor_shape: globals.bind(qh, 1..=2, ()).ok(),
            cursor_device: None,
            registry_state: RegistryState::new(globals),
            output_state: OutputState::new(globals, qh),
            seat_state: SeatState::new(globals, qh),
            overlays: Vec::new(),
            shots: Vec::new(),
            keyboard: None,
            pointer: None,
            active: None,
            press: None,
            current: None,
            result: None,
            exit: false,
        })
    }

    pub fn outputs(&self) -> Vec<wl_output::WlOutput> {
        self.output_state.outputs().collect()
    }

    /// 诊断用：打印每块屏的物理模式、逻辑尺寸与缩放。
    ///
    /// buffer scale 必须整除，否则 layer surface 提交的缓冲尺寸与合成器
    /// 配置的逻辑尺寸对不上，画面会被拉伸甚至触发协议错误。分数缩放
    /// （fractional scaling）就属于对不上的情况。
    pub fn report_outputs(&self) {
        for (i, output) in self.output_state.outputs().enumerate() {
            let Some(info) = self.output_state.info(&output) else {
                println!("屏幕 {i}: 信息不可用");
                continue;
            };
            let mode = info
                .modes
                .iter()
                .find(|m| m.current)
                .map(|m| format!("{}x{}", m.dimensions.0, m.dimensions.1))
                .unwrap_or_else(|| "?".into());
            let logical = info
                .logical_size
                .map(|(w, h)| format!("{w}x{h}"))
                .unwrap_or_else(|| "?".into());
            let integral = match (info.logical_size, info.modes.iter().find(|m| m.current)) {
                (Some((lw, _)), Some(m)) if lw > 0 => {
                    let r = m.dimensions.0 as f64 / lw as f64;
                    if (r - r.round()).abs() < 1e-6 {
                        format!("{}x（整除，OK）", r.round() as i32)
                    } else {
                        format!("{r:.3}x（非整数缩放，layer surface 会对不齐）")
                    }
                }
                _ => "?".into(),
            };
            println!(
                "屏幕 {i}: {} | 物理 {mode} | 逻辑 {logical} | wl_output scale {} | 换算 {integral}",
                info.name.clone().unwrap_or_else(|| "?".into()),
                info.scale_factor
            );
        }
    }

    /// 为每块屏挂一个铺满的浮层。顺序即 `Selection::output_index`。
    pub fn add_overlays(&mut self, qh: &QueueHandle<Self>, shots: Vec<(wl_output::WlOutput, Shot)>) {
        for (output, shot) in shots {
            let name = self
                .output_state
                .info(&output)
                .and_then(|i| i.name)
                .unwrap_or_else(|| "?".into());
            let surface = self.compositor.create_surface(qh);
            let layer = self.layer_shell.create_layer_surface(
                qh,
                surface,
                Layer::Overlay,
                Some("snapocr-shot"),
                Some(&output),
            );
            // 铺满整块屏：四边都锚定，且不为自己预留独占区域。
            layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
            layer.set_exclusive_zone(-1);
            // 独占键盘，这样 Esc 一定到得了我们手里。
            layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
            layer.commit();

            let (bright, dim) = build_render_buffers(&shot);
            self.shots.push(Shot {
                width: shot.width,
                height: shot.height,
                pixels: shot.pixels.clone(),
            });
            self.overlays.push(Overlay {
                layer,
                shot,
                bright,
                dim,
                pool: None,
                last_draw: None,
                name,
                logical: (0, 0),
                configured: false,
            });
        }
    }

    /// 阻塞直到用户框选完成或取消。`None` 表示 Esc 取消或选区无效。
    pub fn run(
        &mut self,
        conn: &Connection,
        queue: &mut wayland_client::EventQueue<Self>,
    ) -> Result<Option<Selection>> {
        while !self.exit {
            queue.blocking_dispatch(self)?;
        }
        // 撤下浮层。对 layer surface 提交 null buffer 属于可疑操作（各合成器行为不一），
        // 正确做法是销毁 surface —— drop 会通过 SCTK 发出 destroy 请求。
        let result = self.result;
        self.overlays.clear();
        if let Err(e) = conn.roundtrip() {
            eprintln!("收尾 roundtrip 失败：{e}");
        }
        Ok(result)
    }

    /// 选区所在屏幕的名字。
    pub fn output_name(&self, sel: &Selection) -> String {
        self.overlays
            .get(sel.output_index)
            .map(|o| o.name.clone())
            .unwrap_or_else(|| "?".into())
    }

    /// 按选区裁出物理像素，输出 RGBA8。
    pub fn crop(&self, sel: &Selection) -> Shot {
        let src = &self.shots[sel.output_index];
        let mut pixels = Vec::with_capacity((sel.width * sel.height * 4) as usize);
        for row in 0..sel.height {
            let y = (sel.y + row) as usize;
            let start = (y * src.width as usize + sel.x as usize) * 4;
            let end = start + sel.width as usize * 4;
            pixels.extend_from_slice(&src.pixels[start..end]);
        }
        Shot { width: sel.width, height: sel.height, pixels }
    }
}

/// 预先算好两份渲染就绪的像素（BGRX，wl_shm 的 Xrgb8888 在小端机上的内存序）。
/// 拖拽时每帧只做整屏拷贝 + 选区回填，不再逐像素做亮度运算。
fn build_render_buffers(shot: &Shot) -> (Vec<u8>, Vec<u8>) {
    let n = (shot.width * shot.height) as usize;
    let mut bright = vec![0u8; n * 4];
    let mut dim = vec![0u8; n * 4];
    for i in 0..n {
        let r = shot.pixels[i * 4] as u32;
        let g = shot.pixels[i * 4 + 1] as u32;
        let b = shot.pixels[i * 4 + 2] as u32;
        bright[i * 4] = b as u8;
        bright[i * 4 + 1] = g as u8;
        bright[i * 4 + 2] = r as u8;
        bright[i * 4 + 3] = 255;
        dim[i * 4] = (b * DIM_KEEP / 255) as u8;
        dim[i * 4 + 1] = (g * DIM_KEEP / 255) as u8;
        dim[i * 4 + 2] = (r * DIM_KEEP / 255) as u8;
        dim[i * 4 + 3] = 255;
    }
    (bright, dim)
}

impl App {
    /// 当前选区，物理像素。仅在 `active` 那块屏上有效。
    fn selection_px(&self) -> Option<(usize, i64, i64, i64, i64)> {
        let idx = self.active?;
        let (sx, sy) = self.press?;
        let (cx, cy) = self.current?;
        let s = self.overlays[idx].scale();
        let x0 = (sx.min(cx) * s).round() as i64;
        let y0 = (sy.min(cy) * s).round() as i64;
        let x1 = (sx.max(cx) * s).round() as i64;
        let y1 = (sy.max(cy) * s).round() as i64;
        Some((idx, x0, y0, x1 - x0, y1 - y0))
    }

    /// 渲染一帧。
    ///
    /// 两条约束：
    /// 1. **缓冲必须复用**。4K 整屏一块缓冲 33MB，每帧新建会让 shm 池一路膨胀
    ///    （高回报率鼠标每秒可产生上千个 motion 事件），最终撑爆合成器。
    /// 2. **必须按帧回调节流**。没有节流就是每个 motion 事件提交一帧，
    ///    远超显示器刷新率，纯属浪费且会加剧上面的问题。
    fn draw(&mut self, index: usize) {
        let sel = self.selection_px().filter(|(i, ..)| *i == index);

        let (w, h) = {
            let o = &self.overlays[index];
            if !o.configured {
                return;
            }
            // 节流：拖拽时高回报率鼠标每秒可产生上千个 motion 事件，
            // 不限速就是每个事件提交一帧，远超刷新率且会让 shm 池堆积。
            //
            // 这里刻意用时间而非 wl_surface 的 frame 回调：回调式节流一旦
            // 收不到回调就会永久卡住（实测 cosmic-comp 上就没回来，表现为
            // 屏幕变暗后选区框再也不刷新），而按时间节流结构上不可能死锁，
            // 跨合成器也更稳。
            if let Some(t) = o.last_draw {
                if t.elapsed() < MIN_FRAME_INTERVAL {
                    return;
                }
            }
            (o.shot.width as i32, o.shot.height as i32)
        };
        // 字号随屏幕缩放走，先算好——下面拿到 pool 的可变借用后就取不到 o 了。
        let glyph = ((self.overlays[index].scale() * 2.0).round() as i64).max(2);
        let stride = w * 4;
        let len = (stride * h) as usize;

        // 池的创建要借 &self.shm，必须赶在下面拿 overlays 的可变借用之前。
        if self.overlays[index].pool.is_none() {
            match SlotPool::new(len, &self.shm) {
                Ok(p) => self.overlays[index].pool = Some(p),
                Err(e) => {
                    eprintln!("分配共享内存失败：{e}");
                    return;
                }
            }
        }

        let o = &mut self.overlays[index];
        let pool = o.pool.as_mut().unwrap();

        // 每帧从池里取缓冲：SlotPool 会复用已被合成器释放的槽位。
        // 不能改成「只建一块反复写」——合成器在收到新缓冲前不会释放旧的，
        // 而我们又因它被占用而跳过绘制，于是选区框再也不刷新（单缓冲死锁）。
        // 真正需要防的是无节制提交，那已由下面的帧回调解决。
        let (buffer, canvas) = match pool.create_buffer(w, h, stride, wl_shm::Format::Xrgb8888) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("创建缓冲区失败：{e}");
                return;
            }
        };

        // 底：整屏压暗。
        canvas[..len].copy_from_slice(&o.dim[..len]);

        // 选区：还原全亮 + 描边。
        if let Some((_, sx, sy, sw, sh)) = sel {
            if sw > 0 && sh > 0 {
                let x0 = sx.clamp(0, w as i64);
                let y0 = sy.clamp(0, h as i64);
                let x1 = (sx + sw).clamp(0, w as i64);
                let y1 = (sy + sh).clamp(0, h as i64);
                for y in y0..y1 {
                    let row = (y * w as i64) as usize * 4;
                    let a = row + x0 as usize * 4;
                    let b = row + x1 as usize * 4;
                    canvas[a..b].copy_from_slice(&o.bright[a..b]);
                }
                draw_border(canvas, w as i64, h as i64, x0, y0, x1, y1);
                draw_size_label(canvas, w as i64, h as i64, x0, y0, x1, sw, sh, glyph);
            }
        }

        let surface = o.layer.wl_surface().clone();
        // buffer scale 必须让 物理缓冲 / scale == 逻辑尺寸，否则画面会被拉伸。
        surface.set_buffer_scale(o.scale().round().max(1.0) as i32);
        surface.damage_buffer(0, 0, w, h);
        if let Err(e) = buffer.attach_to(&surface) {
            eprintln!("attach 缓冲区失败：{e}");
            return;
        }
        surface.commit();
        o.last_draw = Some(std::time::Instant::now());
    }
}

/// 5x7 点阵字模：只要 0-9 和 ×。为画几个数字引入字体光栅化依赖
/// （还要处理字体发现、fontconfig、CJK 回退）不划算。
const GLYPHS: [(char, [u8; 7]); 11] = [
    ('0', [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110]),
    ('1', [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110]),
    ('2', [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111]),
    ('3', [0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110]),
    ('4', [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010]),
    ('5', [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110]),
    ('6', [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110]),
    ('7', [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000]),
    ('8', [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110]),
    ('9', [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100]),
    ('x', [0b00000, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b00000]),
];

/// 在选区上方（放不下则下方）画出实时像素尺寸，如 `1920 x 1080`。
#[allow(clippy::too_many_arguments)]
fn draw_size_label(
    canvas: &mut [u8],
    w: i64,
    h: i64,
    x0: i64,
    y0: i64,
    _x1: i64,
    sel_w: i64,
    sel_h: i64,
    scale: i64,
) {
    let text: Vec<char> = format!("{sel_w} x {sel_h}").chars().collect();
    let pad = 3 * scale;
    let text_w = text.len() as i64 * 6 * scale - scale;
    let box_w = text_w + pad * 2;
    let box_h = 7 * scale + pad * 2;

    // 优先放选区上方；顶部放不下就改到选区内的上沿。
    let bx = x0.min(w - box_w).max(0);
    let by = if y0 - box_h - 2 * scale >= 0 {
        y0 - box_h - 2 * scale
    } else {
        (y0 + 2 * scale).min(h - box_h).max(0)
    };

    // 半透明黑底：直接压暗底色，省掉 alpha 混合。
    for y in by..(by + box_h).min(h) {
        for x in bx..(bx + box_w).min(w) {
            let i = ((y * w + x) * 4) as usize;
            canvas[i] /= 4;
            canvas[i + 1] /= 4;
            canvas[i + 2] /= 4;
        }
    }

    // 白色字形
    let mut cx = bx + pad;
    for ch in text {
        if ch == ' ' {
            cx += 6 * scale;
            continue;
        }
        if let Some((_, rows)) = GLYPHS.iter().find(|(c, _)| *c == ch) {
            for (ry, row) in rows.iter().enumerate() {
                for rx in 0..5i64 {
                    if row & (1 << (4 - rx)) == 0 {
                        continue;
                    }
                    for sy in 0..scale {
                        for sx in 0..scale {
                            let px = cx + rx * scale + sx;
                            let py = by + pad + ry as i64 * scale + sy;
                            if px < 0 || py < 0 || px >= w || py >= h {
                                continue;
                            }
                            let i = ((py * w + px) * 4) as usize;
                            canvas[i] = 255;
                            canvas[i + 1] = 255;
                            canvas[i + 2] = 255;
                        }
                    }
                }
            }
        }
        cx += 6 * scale;
    }
}

fn draw_border(canvas: &mut [u8], w: i64, h: i64, x0: i64, y0: i64, x1: i64, y1: i64) {
    let mut put = |x: i64, y: i64| {
        if x < 0 || y < 0 || x >= w || y >= h {
            return;
        }
        let i = ((y * w + x) * 4) as usize;
        canvas[i] = BORDER_BGR[0];
        canvas[i + 1] = BORDER_BGR[1];
        canvas[i + 2] = BORDER_BGR[2];
    };
    for t in 0..BORDER {
        for x in x0..x1 {
            put(x, y0 + t);
            put(x, y1 - 1 - t);
        }
        for y in y0..y1 {
            put(x0 + t, y);
            put(x1 - 1 - t, y);
        }
    }
}

// ---------------- Wayland 回调 ----------------

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let Some(index) = self.overlays.iter().position(|o| &o.layer == layer) else {
            return;
        };
        let (w, h) = configure.new_size;
        if w != 0 && h != 0 {
            self.overlays[index].logical = (w, h);
        }
        self.overlays[index].configured = true;
        self.draw(index);
    }
}

impl PointerHandler for App {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _qh: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            let Some(index) = self
                .overlays
                .iter()
                .position(|o| o.layer.wl_surface() == &event.surface)
            else {
                continue;
            };
            let (px, py) = event.position;
            match event.kind {
                PointerEventKind::Enter { serial } => {
                    self.active = Some(index);
                    if let Some(dev) = &self.cursor_device {
                        dev.set_shape(serial, wp_cursor_shape_device_v1::Shape::Crosshair);
                    }
                }
                PointerEventKind::Leave { .. } => {
                    // 拖拽中划出屏幕不该丢掉选区，只有没在拖时才清空。
                    if self.press.is_none() && self.active == Some(index) {
                        self.active = None;
                    }
                }
                PointerEventKind::Motion { .. } => {
                    if self.press.is_some() && self.active == Some(index) {
                        self.current = Some((px, py));
                        self.draw(index);
                    }
                }
                PointerEventKind::Press { button, .. } => {
                    // BTN_LEFT
                    if button == 0x110 {
                        self.active = Some(index);
                        self.press = Some((px, py));
                        self.current = Some((px, py));
                    }
                }
                PointerEventKind::Release { button, .. } => {
                    if button == 0x110 && self.press.is_some() {
                        self.current = Some((px, py));
                        if let Some((i, x, y, w, h)) = self.selection_px() {
                            if w >= MIN_SELECTION as i64 && h >= MIN_SELECTION as i64 {
                                let shot = &self.overlays[i].shot;
                                let x = x.clamp(0, shot.width as i64) as u32;
                                let y = y.clamp(0, shot.height as i64) as u32;
                                let w = (w as u32).min(shot.width - x);
                                let h = (h as u32).min(shot.height - y);
                                self.result = Some(Selection {
                                    output_index: i,
                                    x,
                                    y,
                                    width: w,
                                    height: h,
                                });
                            }
                        }
                        self.exit = true;
                    }
                }
                _ => {}
            }
        }
    }
}

impl KeyboardHandler for App {
    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        if event.keysym == Keysym::Escape {
            self.result = None;
            self.exit = true;
        }
    }

    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: Modifiers,
        _: RawModifiers,
        _: u32,
    ) {
    }

    fn repeat_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Keyboard if self.keyboard.is_none() => {
                if let Ok(kb) = self.seat_state.get_keyboard(qh, &seat, None) {
                    self.keyboard = Some(kb);
                }
            }
            Capability::Pointer if self.pointer.is_none() => {
                if let Ok(p) = self.seat_state.get_pointer(qh, &seat) {
                    if let Some(mgr) = &self.cursor_shape {
                        self.cursor_device = Some(mgr.get_pointer(&p, qh, ()));
                    }
                    self.pointer = Some(p);
                }
            }
            _ => {}
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        _: Capability,
    ) {
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

impl Dispatch<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &wp_cursor_shape_manager_v1::WpCursorShapeManagerV1,
        _: wp_cursor_shape_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &wp_cursor_shape_device_v1::WpCursorShapeDeviceV1,
        _: wp_cursor_shape_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_registry!(App);
smithay_client_toolkit::delegate_dispatch2!(App);
