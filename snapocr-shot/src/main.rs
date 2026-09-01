//! snapocr-shot —— 冻结抓屏 + 框选浮层，输出 PNG。
//!
//! 为 cosmic-comp 这类只实现 ext-image-copy-capture 的现代合成器补上 grim+slurp 的空缺：
//! 抓下每块屏的当前画面 → 全屏压暗浮层 → 用户拖框 → 裁剪输出 PNG。
//!
//! 用法：
//!     snapocr-shot [输出路径]     省略或写 `-` 则输出到 stdout
//!
//! 退出码：0 成功，1 出错，2 用户取消（Esc 或未框选），3 看门狗超时。

mod capture;
mod overlay;

use anyhow::{Context, Result};
use std::io::Write;
use wayland_client::{globals::registry_queue_init, Connection};

/// 浮层独占键盘并铺满全屏。万一事件循环卡死，用户将无法操作桌面，
/// 所以设一个硬性上限强制退出——宁可截图失败，不可让桌面失去响应。
/// 可用 SNAPOCR_TIMEOUT 覆盖（首次试跑时调短些更稳妥）。
const WATCHDOG_SECS: u64 = 60;

fn watchdog_secs() -> u64 {
    std::env::var("SNAPOCR_TIMEOUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(WATCHDOG_SECS)
}

fn main() {
    match run() {
        Ok(true) => {}
        Ok(false) => {
            eprintln!("已取消");
            std::process::exit(2);
        }
        Err(e) => {
            eprintln!("错误：{e:#}");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<bool> {
    let out_path = std::env::args().nth(1).unwrap_or_else(|| "-".into());

    let timeout = watchdog_secs();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(timeout));
        eprintln!("看门狗超时（{timeout}s），强制退出以免浮层卡住桌面");
        std::process::exit(3);
    });

    let conn = Connection::connect_to_env().context("连接 Wayland 合成器失败")?;
    let (globals, mut queue) = registry_queue_init::<overlay::App>(&conn)?;
    let qh = queue.handle();

    let mut app = overlay::App::new(&globals, &qh)?;
    // 让 OutputState 收齐 output 信息。两次 roundtrip：第一次拿到 wl_output，
    // 第二次才收齐它们的 mode/scale/logical_size 事件。
    queue.roundtrip(&mut app)?;
    queue.roundtrip(&mut app)?;

    if std::env::args().any(|a| a == "--outputs") {
        app.report_outputs();
        return Ok(true);
    }

    let outputs = app.outputs();
    if outputs.is_empty() {
        anyhow::bail!("没有找到任何屏幕");
    }

    // 先把所有屏冻结下来，再显示浮层——顺序反了就会把浮层自己拍进去。
    let mut shots = Vec::with_capacity(outputs.len());
    for (i, output) in outputs.iter().enumerate() {
        let shot = capture::capture_output(&conn, &globals, output)
            .with_context(|| format!("抓取第 {} 块屏失败", i + 1))?;
        shots.push((output.clone(), shot));
    }

    app.add_overlays(&qh, shots);
    let Some(sel) = app.run(&conn, &mut queue)? else {
        return Ok(false);
    };

    let cropped = app.crop(&sel);
    eprintln!(
        "选区：{} 上 {}x{} @ ({}, {})",
        app.output_name(&sel),
        cropped.width,
        cropped.height,
        sel.x,
        sel.y
    );

    if out_path == "-" {
        let stdout = std::io::stdout();
        let mut w = std::io::BufWriter::new(stdout.lock());
        write_png(&mut w, &cropped)?;
        w.flush()?;
    } else {
        let file = std::fs::File::create(&out_path)
            .with_context(|| format!("无法写入 {out_path}"))?;
        let mut w = std::io::BufWriter::new(file);
        write_png(&mut w, &cropped)?;
        w.flush()?;
    }
    Ok(true)
}

fn write_png<W: Write>(w: W, shot: &capture::Shot) -> Result<()> {
    let mut enc = png::Encoder::new(w, shot.width, shot.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&shot.pixels)?;
    Ok(())
}
