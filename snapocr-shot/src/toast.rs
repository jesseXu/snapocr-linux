//! 绘制工具箱：像素混合、圆角矩形、线段、点阵数字、图标。
//!
//! 为什么自己画 toast 而不用桌面通知：`org.freedesktop.Notifications` 虽是
//! 标准，但「渲染动作按钮」各家实现差异极大 —— cosmic-notifications 实测
//! 不画，dunst 的包描述直说「只显示一个纯文本色块」，mako 靠点击整条通知
//! 触发。指望通知按钮反而是最不通用的做法。而本工具**已经**硬依赖
//! layer-shell（框选浮层就是 layer surface），自绘的可移植性成本是零。
//!
//! 为什么用图标而不是文字：图标不需要翻译，也就不需要字体发现、排版和
//! CJK 回退那一整套（原先为此引入过 cosmic-text）。toast 上要表达的东西
//! ——「复制好了」「存好了」「按 S」「按 E」——都能用几笔线条画清楚，
//! 唯一的文字是尺寸数字，点阵字模足矣。

/// 画布：BGRA，行优先。与 wl_shm 的 8888 格式在小端机上的内存序一致。
pub struct Canvas<'a> {
    pub data: &'a mut [u8],
    pub width: i32,
    pub height: i32,
}

impl Canvas<'_> {
    pub fn blend(&mut self, x: i32, y: i32, rgb: (u8, u8, u8), alpha: u8) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height || alpha == 0 {
            return;
        }
        let i = ((y * self.width + x) * 4) as usize;
        let (a, inv) = (alpha as u32, 255 - alpha as u32);
        for (offset, channel) in [(0, rgb.2), (1, rgb.1), (2, rgb.0)] {
            let dst = self.data[i + offset] as u32;
            self.data[i + offset] = ((channel as u32 * a + dst * inv) / 255) as u8;
        }
        let dst_a = self.data[i + 3] as u32;
        self.data[i + 3] = (a + dst_a * inv / 255).min(255) as u8;
    }
}

// ---------- 基本图元 ----------

/// 圆角矩形填充，圆角处做一像素抗锯齿（2x 屏上不做的话锯齿很明显）。
pub fn fill_rounded_rect(
    c: &mut Canvas, x: i32, y: i32, w: i32, h: i32, radius: f32,
    rgb: (u8, u8, u8), alpha: u8,
) {
    for py in y..(y + h) {
        for px in x..(x + w) {
            let cov = rounded_coverage(px - x, py - y, w, h, radius);
            if cov > 0.0 {
                c.blend(px, py, rgb, (alpha as f32 * cov) as u8);
            }
        }
    }
}

/// 圆角矩形描边：外圈减内圈。
pub fn stroke_rounded_rect(
    c: &mut Canvas, x: i32, y: i32, w: i32, h: i32, radius: f32, thickness: f32,
    rgb: (u8, u8, u8), alpha: u8,
) {
    let t = thickness.max(1.0);
    for py in y..(y + h) {
        for px in x..(x + w) {
            let outer = rounded_coverage(px - x, py - y, w, h, radius);
            let inner = rounded_coverage(
                px - x - t as i32, py - y - t as i32,
                w - 2 * t as i32, h - 2 * t as i32, (radius - t).max(0.0),
            );
            let cov = (outer - inner).clamp(0.0, 1.0);
            if cov > 0.0 {
                c.blend(px, py, rgb, (alpha as f32 * cov) as u8);
            }
        }
    }
}

fn rounded_coverage(lx: i32, ly: i32, w: i32, h: i32, radius: f32) -> f32 {
    if w <= 0 || h <= 0 {
        return 0.0;
    }
    let (fx, fy) = (lx as f32 + 0.5, ly as f32 + 0.5);
    if fx < 0.0 || fy < 0.0 || fx > w as f32 || fy > h as f32 {
        return 0.0;
    }
    let cx = if fx < radius { radius }
        else if fx > w as f32 - radius { w as f32 - radius }
        else { return 1.0 };
    let cy = if fy < radius { radius }
        else if fy > h as f32 - radius { h as f32 - radius }
        else { return 1.0 };
    let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
    (radius + 0.5 - d).clamp(0.0, 1.0)
}

/// 带抗锯齿的粗线段。图标全部由它拼出来，所以值得写好。
pub fn stroke_line(
    c: &mut Canvas, x0: f32, y0: f32, x1: f32, y1: f32, width: f32,
    rgb: (u8, u8, u8), alpha: u8,
) {
    let half = width / 2.0;
    let pad = half.ceil() + 1.0;
    let (lo_x, hi_x) = (x0.min(x1) - pad, x0.max(x1) + pad);
    let (lo_y, hi_y) = (y0.min(y1) - pad, y0.max(y1) + pad);
    for py in lo_y.floor() as i32..=hi_y.ceil() as i32 {
        for px in lo_x.floor() as i32..=hi_x.ceil() as i32 {
            let d = point_segment_distance(px as f32 + 0.5, py as f32 + 0.5, x0, y0, x1, y1);
            let cov = (half + 0.5 - d).clamp(0.0, 1.0);
            if cov > 0.0 {
                c.blend(px, py, rgb, (alpha as f32 * cov) as u8);
            }
        }
    }
}

fn point_segment_distance(px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32) -> f32 {
    let (dx, dy) = (x1 - x0, y1 - y0);
    let len_sq = dx * dx + dy * dy;
    let t = if len_sq <= f32::EPSILON {
        0.0
    } else {
        (((px - x0) * dx + (py - y0) * dy) / len_sq).clamp(0.0, 1.0)
    };
    ((px - (x0 + t * dx)).powi(2) + (py - (y0 + t * dy)).powi(2)).sqrt()
}

/// 多边形填充，4x4 超采样抗锯齿。
///
/// 小图标（18~20px）上细线条会显得很虚，实心形状才读得清 —— 这是把
/// toast 图标重画一遍的主要原因。
pub fn fill_polygon(c: &mut Canvas, pts: &[(f32, f32)], rgb: (u8, u8, u8), alpha: u8) {
    if pts.len() < 3 {
        return;
    }
    let min_x = pts.iter().map(|p| p.0).fold(f32::MAX, f32::min).floor() as i32;
    let max_x = pts.iter().map(|p| p.0).fold(f32::MIN, f32::max).ceil() as i32;
    let min_y = pts.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor() as i32;
    let max_y = pts.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil() as i32;
    const N: i32 = 4;
    for py in min_y..=max_y {
        for px in min_x..=max_x {
            let mut hits = 0;
            for sy in 0..N {
                for sx in 0..N {
                    let x = px as f32 + (sx as f32 + 0.5) / N as f32;
                    let y = py as f32 + (sy as f32 + 0.5) / N as f32;
                    if point_in_polygon(x, y, pts) {
                        hits += 1;
                    }
                }
            }
            if hits > 0 {
                let cov = hits as f32 / (N * N) as f32;
                c.blend(px, py, rgb, (alpha as f32 * cov) as u8);
            }
        }
    }
}

fn point_in_polygon(x: f32, y: f32, pts: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let mut j = pts.len() - 1;
    for i in 0..pts.len() {
        let (xi, yi) = pts[i];
        let (xj, yj) = pts[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

// ---------- 点阵字模 ----------

/// 5x7 点阵，只覆盖 toast 与尺寸标签实际用到的字符。
/// 为画这几个字符引入字体光栅化（还要处理字体发现、fontconfig、CJK 回退）
/// 不划算 —— 这也是把文案图标化之后最大的收益。
const GLYPHS: [(char, [u8; 7]); 13] = [
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
    ('S', [0b01110, 0b10001, 0b10000, 0b01110, 0b00001, 0b10001, 0b01110]),
    ('E', [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111]),
];

/// 一个字符占 5 列 + 1 列间距。
pub fn text_width(text: &str, scale: i32) -> i32 {
    (text.chars().count() as i32 * 6 - 1) * scale
}

pub fn draw_text(c: &mut Canvas, text: &str, x: i32, y: i32, scale: i32, rgb: (u8, u8, u8), alpha: u8) {
    let mut cx = x;
    for ch in text.chars() {
        if let Some((_, rows)) = GLYPHS.iter().find(|(g, _)| *g == ch) {
            for (ry, row) in rows.iter().enumerate() {
                for rx in 0..5i32 {
                    if row & (1 << (4 - rx)) == 0 {
                        continue;
                    }
                    for sy in 0..scale {
                        for sx in 0..scale {
                            c.blend(cx + rx * scale + sx, y + ry as i32 * scale + sy, rgb, alpha);
                        }
                    }
                }
            }
        }
        cx += 6 * scale;
    }
}

// ---------- 图标 ----------
//
// 全部按「在 (x, y) 处画一个 size x size 的图标」约定，内部用相对比例，
// 这样在任意缩放下都成比例。

const W: (u8, u8, u8) = (255, 255, 255);

/// 两个交叠的圆角方块：已复制。
///
/// 原先画的是「剪贴板 + 对勾」，在 26px 里挤了圆角框、顶部夹子、对勾三样，
/// 形状毛糙，紧挨着一串数字很不协调，含义也不直给。交叠方块是各家 UI 通用
/// 的复制符号，只有两个几何体，和数字并排也稳。
pub fn icon_copy(c: &mut Canvas, x: f32, y: f32, size: f32, rgb: (u8, u8, u8), alpha: u8) {
    let t = (size * 0.11).max(1.5);
    let side = size * 0.62;
    // 后面那张：只描边，露出左上一角
    stroke_rounded_rect(
        c, (x + size * 0.02) as i32, (y + size * 0.02) as i32,
        side as i32, side as i32, size * 0.14, t, rgb, alpha,
    );
    // 前面那张：先挖掉与后者重叠的部分，再描边，交叠关系才看得出来
    let fx = x + size * 0.36;
    let fy = y + size * 0.36;
    fill_rounded_rect(
        c, (fx - t) as i32, (fy - t) as i32,
        (side + t * 2.0) as i32, (side + t * 2.0) as i32,
        size * 0.16, (18, 18, 20), 255,
    );
    stroke_rounded_rect(
        c, fx as i32, fy as i32, side as i32, side as i32,
        size * 0.14, t, rgb, alpha,
    );
}

/// 向下箭头落到托盘上：保存到磁盘。箭头头部用实心三角 —— 细线画的
/// 人字形箭头在 18px 上几乎看不出方向。
pub fn icon_save(c: &mut Canvas, x: f32, y: f32, size: f32, rgb: (u8, u8, u8), alpha: u8) {
    let t = (size * 0.16).max(2.0);
    let cx = x + size / 2.0;
    stroke_line(c, cx, y + size * 0.08, cx, y + size * 0.42, t, rgb, alpha);
    fill_polygon(c, &[
        (cx - size * 0.26, y + size * 0.38),
        (cx + size * 0.26, y + size * 0.38),
        (cx, y + size * 0.68),
    ], rgb, alpha);
    // 托盘：一条粗横线加两个小立边，足够表达「落到某处」
    let tt = (size * 0.14).max(2.0);
    stroke_line(c, x + size * 0.10, y + size * 0.88, x + size * 0.90, y + size * 0.88, tt, rgb, alpha);
    stroke_line(c, x + size * 0.10, y + size * 0.70, x + size * 0.10, y + size * 0.88, tt, rgb, alpha);
    stroke_line(c, x + size * 0.90, y + size * 0.70, x + size * 0.90, y + size * 0.88, tt, rgb, alpha);
}

/// 一支铅笔：标注。整支笔用实心多边形而非线条 —— 细线画的笔在 18px 上
/// 只是一道斜杠，看不出是笔。
pub fn icon_pen(c: &mut Canvas, x: f32, y: f32, size: f32, rgb: (u8, u8, u8), alpha: u8) {
    let s = size;
    // 笔身：沿左下→右上的斜向四边形
    fill_polygon(c, &[
        (x + s * 0.34, y + s * 0.80),
        (x + s * 0.20, y + s * 0.66),
        (x + s * 0.66, y + s * 0.20),
        (x + s * 0.80, y + s * 0.34),
    ], rgb, alpha);
    // 笔尖：指向左下的实心三角
    fill_polygon(c, &[
        (x + s * 0.20, y + s * 0.66),
        (x + s * 0.34, y + s * 0.80),
        (x + s * 0.12, y + s * 0.88),
    ], rgb, alpha);
    // 笔帽端的分隔线，让它更像铅笔而不是一块菱形
    stroke_line(c, x + s * 0.58, y + s * 0.28, x + s * 0.72, y + s * 0.42,
                (s * 0.07).max(1.0), (18, 18, 20), alpha);
}

/// 对勾：已保存。
pub fn icon_check(c: &mut Canvas, x: f32, y: f32, size: f32, rgb: (u8, u8, u8), alpha: u8) {
    let t = (size * 0.12).max(1.0);
    stroke_line(c, x + size * 0.18, y + size * 0.52, x + size * 0.42, y + size * 0.76, t, rgb, alpha);
    stroke_line(c, x + size * 0.42, y + size * 0.76, x + size * 0.84, y + size * 0.24, t, rgb, alpha);
}

/// 键帽：圆角描边框 + 里面一个字母。返回它占的宽度。
pub fn draw_keycap(c: &mut Canvas, letter: &str, x: f32, y: f32, size: f32, alpha: u8) -> f32 {
    // 字号由键帽高度反推：点阵是 7 行高，让字占键帽约一半高度。
    // （写成 size * 0.30 会让字比键帽还大 —— 那是按「字号」而非「行数」算的。）
    let scale = ((size * 0.5 / 7.0).round() as i32).max(1);
    let w = (size * 0.86).max(text_width(letter, scale) as f32 + size * 0.36);
    stroke_rounded_rect(
        c, x as i32, y as i32, w as i32, size as i32,
        size * 0.22, (size * 0.07).max(1.0), (180, 180, 190), alpha,
    );
    let tw = text_width(letter, scale);
    let th = 7 * scale;
    draw_text(
        c, letter,
        (x + (w - tw as f32) / 2.0) as i32,
        (y + (size - th as f32) / 2.0) as i32,
        scale, W, alpha,
    );
    w
}
