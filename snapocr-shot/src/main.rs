//! snapocr-shot —— 冻结抓屏 + 框选浮层，输出 PNG。
//!
//! 为 cosmic-comp 这类只实现 ext-image-copy-capture 的现代合成器补上 grim+slurp 的空缺：
//! 抓下每块屏的当前画面 → 全屏压暗浮层 → 用户拖框 → 裁剪输出 PNG。
//!
//! 用法：
//!     snapocr-shot [输出路径]        框选，省略路径或写 `-` 则输出到 stdout
//!     snapocr-shot --outputs         打印各屏尺寸与缩放
//!     snapocr-shot --full [目录]     非交互整屏抓取（诊断用）
//!     snapocr-shot --toast --state copied|saved [--body "1169 x 651"] [--timeout MS]
//!                                    底部弹一条浮层，用退出码回报用户按了什么
//!
//! 退出码：0 成功/无动作，1 出错，2 用户取消，3 看门狗超时，
//!         10 toast 上按了 S，11 toast 上按了 E。

mod capture;
mod overlay;
mod toast;

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
            eprintln!("Cancelled");
            std::process::exit(2);
        }
        Err(e) => {
            eprintln!("Error: {e:#}");
            std::process::exit(1);
        }
    }
}

fn arg_value(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn run() -> Result<bool> {
    let out_path = std::env::args().nth(1).unwrap_or_else(|| "-".into());

    let timeout = watchdog_secs();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(timeout));
        eprintln!("Watchdog timeout ({timeout}s) — exiting so the overlay cannot lock up the desktop");
        std::process::exit(3);
    });

    let conn = Connection::connect_to_env().context("could not connect to the Wayland compositor")?;
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

    if std::env::args().any(|a| a == "--toast") {
        let timeout_ms: u64 = arg_value("--timeout")
            .and_then(|v| v.parse().ok())
            .unwrap_or(4000);
        // 到点直接退进程。toast 模式没有任何待落盘的东西，这么做是安全的，
        // 也省掉了给事件循环加一套定时器的复杂度。
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(timeout_ms));
            std::process::exit(0);
        });
        let copied = arg_value("--state").as_deref() != Some("saved");
        app.add_toast(&qh, copied, &arg_value("--body").unwrap_or_default());
        app.run(&conn, &mut queue)?;
        std::process::exit(app.action as i32);
    }

    // 非交互整屏抓取：不显示浮层、不需要任何操作。用来做诊断，
    // 也让「看一眼当前屏幕是什么样」这件事可脚本化。
    if std::env::args().any(|a| a == "--full") {
        let dir = std::env::args()
            .nth(1)
            .filter(|a| a != "--full")
            .unwrap_or_else(|| ".".into());
        for (output, name) in &app.outputs() {
            let shot = capture::capture_output(&conn, &globals, output)
                .with_context(|| format!("failed to capture output {name}"))?;
            let path = format!("{dir}/{name}.png");
            let file = std::fs::File::create(&path)?;
            write_png(std::io::BufWriter::new(file), &shot)?;
            println!("{path}  ({}x{})", shot.width, shot.height);
        }
        return Ok(true);
    }

    let outputs = app.outputs();
    if outputs.is_empty() {
        anyhow::bail!("no outputs found");
    }

    // 先把所有屏冻结下来，再显示浮层——顺序反了就会把浮层自己拍进去。
    let mut shots = Vec::with_capacity(outputs.len());
    for (output, name) in &outputs {
        let shot = capture::capture_output(&conn, &globals, output)
            .with_context(|| format!("failed to capture output {name}"))?;
        shots.push((output.clone(), name.clone(), shot));
    }

    app.add_overlays(&qh, shots);
    let Some(sel) = app.run(&conn, &mut queue)? else {
        return Ok(false);
    };

    let cropped = app.crop(&sel);
    eprintln!(
        "selection: {}x{} at ({}, {}) on {}",
        cropped.width,
        cropped.height,
        sel.x,
        sel.y,
        app.output_name(&sel)
    );

    if out_path == "-" {
        let stdout = std::io::stdout();
        let mut w = std::io::BufWriter::new(stdout.lock());
        write_png(&mut w, &cropped)?;
        w.flush()?;
    } else {
        let file = std::fs::File::create(&out_path)
            .with_context(|| format!("cannot write {out_path}"))?;
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
