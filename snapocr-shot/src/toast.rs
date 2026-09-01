//! toast 的绘制：圆角深色底 + 文字。
//!
//! 为什么自己画而不用桌面通知：`org.freedesktop.Notifications` 虽是标准，
//! 但「渲染动作按钮」各家实现差异极大 —— cosmic-notifications 实测不画，
//! dunst 的包描述直说「只显示一个纯文本色块」，mako 靠点击整条通知触发。
//! 也就是说，指望通知按钮反而是最不通用的做法。
//!
//! 而本工具**已经**硬依赖 layer-shell（框选浮层就是 layer surface），
//! layer-shell 跑不了的桌面上整个工具本来就跑不起来 —— 所以自绘 toast
//! 的可移植性成本是零，还换来了跨桌面完全一致的观感与按键。

use cosmic_text::{Attrs, Buffer, Color, FontSystem, Metrics, Shaping, SwashCache};

/// 画布：BGRX，行优先。与 wl_shm 的 Xrgb8888 在小端机上的内存序一致。
pub struct Canvas<'a> {
    pub data: &'a mut [u8],
    pub width: i32,
    pub height: i32,
}

impl Canvas<'_> {
    fn blend(&mut self, x: i32, y: i32, rgb: (u8, u8, u8), alpha: u8) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height || alpha == 0 {
            return;
        }
        let i = ((y * self.width + x) * 4) as usize;
        let a = alpha as u32;
        let inv = 255 - a;
        // 目标是 BGRX，注意通道顺序。
        for (offset, channel) in [(0, rgb.2), (1, rgb.1), (2, rgb.0)] {
            let dst = self.data[i + offset] as u32;
            self.data[i + offset] = ((channel as u32 * a + dst * inv) / 255) as u8;
        }
        self.data[i + 3] = 255;
    }
}

/// 圆角矩形填充。半径处按到圆心的距离做一像素的抗锯齿，
/// 否则在 2x 屏上圆角的锯齿相当明显。
pub fn fill_rounded_rect(
    canvas: &mut Canvas,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    radius: f32,
    rgb: (u8, u8, u8),
    alpha: u8,
) {
    for py in y..(y + h) {
        for px in x..(x + w) {
            let coverage = corner_coverage(px - x, py - y, w, h, radius);
            if coverage <= 0.0 {
                continue;
            }
            canvas.blend(px, py, rgb, (alpha as f32 * coverage) as u8);
        }
    }
}

/// 返回该像素落在圆角矩形内的比例（0..1）。只有四个角需要计算。
fn corner_coverage(lx: i32, ly: i32, w: i32, h: i32, radius: f32) -> f32 {
    let (fx, fy) = (lx as f32 + 0.5, ly as f32 + 0.5);
    let (cx, cy) = (
        if fx < radius {
            radius
        } else if fx > w as f32 - radius {
            w as f32 - radius
        } else {
            return 1.0;
        },
        if fy < radius {
            radius
        } else if fy > h as f32 - radius {
            h as f32 - radius
        } else {
            return 1.0;
        },
    );
    let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
    (radius + 0.5 - d).clamp(0.0, 1.0)
}

/// 文字渲染器。`FontSystem` 的构造会扫描系统字体，比较慢，
/// 所以整个进程共用一个。
pub struct TextRenderer {
    fonts: FontSystem,
    cache: SwashCache,
}

impl TextRenderer {
    pub fn new() -> Self {
        Self {
            fonts: FontSystem::new(),
            cache: SwashCache::new(),
        }
    }

    /// 量出一行文字的像素宽度。
    pub fn measure(&mut self, text: &str, size: f32) -> f32 {
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, size * 1.35));
        buffer.set_size(Some(f32::INFINITY), Some(size * 2.0));
        buffer.set_text(text, &Attrs::new(), Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0_f32, f32::max)
    }

    /// 在 (x, y) 处绘制一行文字（y 为文字块顶部）。
    /// `Shaping::Advanced` 是中文正确显示的前提。
    pub fn draw(
        &mut self,
        canvas: &mut Canvas,
        text: &str,
        x: f32,
        y: f32,
        size: f32,
        rgb: (u8, u8, u8),
        alpha: u8,
    ) {
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, size * 1.35));
        buffer.set_size(Some(f32::INFINITY), Some(size * 2.0));
        buffer.set_text(text, &Attrs::new(), Shaping::Advanced, None);
        let base = Color::rgb(rgb.0, rgb.1, rgb.2);
        buffer.draw(
            &mut self.fonts,
            &mut self.cache,
            base,
            |gx, gy, gw, gh, color| {
                // 字形的覆盖度在 color 的 alpha 里，乘上整体透明度。
                let a = (color.a() as u32 * alpha as u32 / 255) as u8;
                for dy in 0..gh as i32 {
                    for dx in 0..gw as i32 {
                        canvas.blend(x as i32 + gx + dx, y as i32 + gy + dy, rgb, a);
                    }
                }
            },
        );
    }
}
