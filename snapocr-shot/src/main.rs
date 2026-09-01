//! snapocr-shot —— 冻结抓屏 + 框选浮层，输出 PNG。
//!
//! 为 cosmic-comp 这类只实现 ext-image-copy-capture 的现代合成器补上 grim+slurp 的空缺。
//! 当前进度：抓屏已通，框选浮层待接。

mod capture;

use anyhow::{Context, Result};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{Connection, Dispatch, QueueHandle};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out_dir = args
        .iter()
        .position(|a| a == "-o")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| ".".into());

    let conn = Connection::connect_to_env().context("连接 Wayland 合成器失败")?;
    let (globals, mut queue) = registry_queue_init::<Noop>(&conn)?;
    let qh = queue.handle();

    // 枚举所有 output（这台机器有两块屏，从一开始就按 N 块处理）。
    let outputs: Vec<wl_output::WlOutput> = globals
        .contents()
        .clone_list()
        .into_iter()
        .filter(|g| g.interface == "wl_output")
        .map(|g| {
            globals
                .registry()
                .bind::<wl_output::WlOutput, _, _>(g.name, g.version.min(4), &qh, ())
        })
        .collect();
    queue.roundtrip(&mut Noop)?;

    if outputs.is_empty() {
        anyhow::bail!("没有找到任何 wl_output");
    }
    println!("找到 {} 块屏幕", outputs.len());

    for (i, output) in outputs.iter().enumerate() {
        let t = std::time::Instant::now();
        let shot = capture::capture_output(&conn, &globals, output)
            .with_context(|| format!("抓取第 {} 块屏失败", i + 1))?;
        let elapsed = t.elapsed();

        let path = format!("{out_dir}/snapocr-output{i}.png");
        write_png(&path, &shot)?;
        println!(
            "  屏幕 {}: {}x{}  用时 {:?}  ->  {}",
            i, shot.width, shot.height, elapsed, path
        );
    }
    Ok(())
}

fn write_png(path: &str, shot: &capture::Shot) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let w = std::io::BufWriter::new(file);
    let mut enc = png::Encoder::new(w, shot.width, shot.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&shot.pixels)?;
    Ok(())
}

/// registry_queue_init 需要一个 state；这里的对象我们不关心事件。
struct Noop;

impl Dispatch<wl_output::WlOutput, ()> for Noop {
    fn event(
        _: &mut Self,
        _: &wl_output::WlOutput,
        _: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Noop {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
