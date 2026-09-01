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
use wayland_client::{
    globals::GlobalList,
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

    overlays: Vec<Overlay>,
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
            registry_state: RegistryState::new(globals),
            output_state: OutputState::new(globals, qh),
            seat_state: SeatState::new(globals, qh),
            overlays: Vec::new(),
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

    /// 为每块屏挂一个铺满的浮层。顺序即 `Selection::output_index`。
    pub fn add_overlays(&mut self, qh: &QueueHandle<Self>, shots: Vec<(wl_output::WlOutput, Shot)>) {
        for (output, shot) in shots {
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
            self.overlays.push(Overlay {
                layer,
                shot,
                bright,
                dim,
                pool: None,
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
        // 明确撤下浮层，别让它在进程收尾期间还挂在屏幕上。
        for o in &self.overlays {
            o.layer.wl_surface().attach(None, 0, 0);
            o.layer.wl_surface().commit();
        }
        let _ = conn.roundtrip();
        Ok(self.result)
    }

    /// 按选区裁出物理像素，输出 RGBA8。
    pub fn crop(&self, sel: &Selection) -> Shot {
        let src = &self.overlays[sel.output_index].shot;
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

    fn draw(&mut self, index: usize) {
        let sel = self.selection_px().filter(|(i, ..)| *i == index);
        let o = &mut self.overlays[index];
        if !o.configured {
            return;
        }
        let (w, h) = (o.shot.width as i32, o.shot.height as i32);
        let stride = w * 4;
        let len = (stride * h) as usize;

        if o.pool.is_none() {
            match SlotPool::new(len, &self.shm) {
                Ok(p) => o.pool = Some(p),
                Err(e) => {
                    eprintln!("分配共享内存失败：{e}");
                    return;
                }
            }
        }
        let pool = o.pool.as_mut().unwrap();
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
            }
        }

        let surface = o.layer.wl_surface();
        // buffer scale 必须让 物理缓冲 / scale == 逻辑尺寸，否则画面会被拉伸。
        let bs = o.scale().round().max(1.0) as i32;
        surface.set_buffer_scale(bs);
        surface.damage_buffer(0, 0, w, h);
        // 不注册 frame 回调：拖拽时直接按指针事件重绘。合成器只取最新一次提交，
        // 多提交的代价远小于漏一帧带来的拖影。
        if let Err(e) = buffer.attach_to(surface) {
            eprintln!("attach 缓冲区失败：{e}");
            return;
        }
        surface.commit();
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
                PointerEventKind::Enter { .. } => {
                    self.active = Some(index);
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

delegate_registry!(App);
smithay_client_toolkit::delegate_dispatch2!(App);
