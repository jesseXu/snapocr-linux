//! 冻结抓屏：ext-image-copy-capture-v1。
//!
//! 这是 freedesktop 的**标准**抓屏协议（`ext_` 前缀），取代了 wlroots 私有的
//! `zwlr_screencopy_manager_v1`。cosmic-comp 只实现前者，所以 grim 在 COSMIC 上不可用。
//!
//! 协议流程：
//!   source_manager.create_source(output)  -> ext_image_capture_source_v1
//!   capture_manager.create_session(src)   -> session
//!   session 依次发 buffer_size / shm_format / dmabuf_* ，最后 done
//!   session.create_frame()                -> frame
//!   frame.attach_buffer + damage_buffer + capture
//!   frame 回 ready（成功）或 failed
//!
//! 我们只走 shm 路径：一次性截图不值得为 dmabuf 引入 GPU 依赖。

use anyhow::{bail, Context, Result};
use wayland_client::globals::GlobalList;
use wayland_client::protocol::{wl_buffer, wl_output, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_image_capture_source_v1, ext_output_image_capture_source_manager_v1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1, ext_image_copy_capture_manager_v1,
    ext_image_copy_capture_session_v1,
};

/// 一块屏幕的冻结像素，RGBA8，行优先，无 padding。
pub struct Shot {
    pub width: u32,
    pub height: u32,
    /// len == width * height * 4
    pub pixels: Vec<u8>,
}

#[derive(Default)]
struct State {
    buffer_size: Option<(u32, u32)>,
    shm_formats: Vec<wl_shm::Format>,
    session_done: bool,
    frame_ready: bool,
    frame_failed: Option<String>,
}

/// 抓取指定 output 的当前画面。阻塞直到合成器回 ready 或 failed。
pub fn capture_output(
    conn: &Connection,
    globals: &GlobalList,
    output: &wl_output::WlOutput,
) -> Result<Shot> {
    let mut queue = conn.new_event_queue::<State>();
    let qh = queue.handle();

    let source_mgr: ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1 =
        globals
            .bind(&qh, 1..=1, ())
            .context("合成器没有 ext_output_image_capture_source_manager_v1")?;
    let capture_mgr: ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1 = globals
        .bind(&qh, 1..=1, ())
        .context("合成器没有 ext_image_copy_capture_manager_v1")?;
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=2, ()).context("合成器没有 wl_shm")?;

    let source = source_mgr.create_source(output, &qh, ());
    // Options::empty() = 不绘制光标。截图里不要鼠标指针。
    let session = capture_mgr.create_session(
        &source,
        ext_image_copy_capture_manager_v1::Options::empty(),
        &qh,
        (),
    );

    // 等 session 把 buffer_size / format 协商完（以 done 收尾）。
    let mut state = State::default();
    while !state.session_done {
        queue
            .blocking_dispatch(&mut state)
            .context("等待 capture session 协商失败")?;
        if let Some(reason) = &state.frame_failed {
            bail!("抓屏 session 被合成器中止：{reason}");
        }
    }

    let (width, height) = state
        .buffer_size
        .context("合成器未提供 buffer_size，无法分配缓冲区")?;
    let format = pick_format(&state.shm_formats)?;

    let stride = width * 4;
    let len = (stride * height) as usize;

    // 用 memfd 做共享内存：合成器直接往里写像素。
    let file = memfd_of_size(len).context("创建共享内存失败")?;
    let mmap = unsafe { memmap2::MmapMut::map_mut(&file)? };
    let pool = shm.create_pool(
        std::os::fd::AsFd::as_fd(&file),
        len as i32,
        &qh,
        (),
    );
    let buffer = pool.create_buffer(
        0,
        width as i32,
        height as i32,
        stride as i32,
        format,
        &qh,
        (),
    );

    let frame = session.create_frame(&qh, ());
    frame.attach_buffer(&buffer);
    frame.damage_buffer(0, 0, width as i32, height as i32);
    frame.capture();

    while !state.frame_ready && state.frame_failed.is_none() {
        queue
            .blocking_dispatch(&mut state)
            .context("等待抓屏帧失败")?;
    }
    if let Some(reason) = state.frame_failed {
        bail!("抓屏失败：{reason}");
    }

    let pixels = to_rgba(&mmap[..len], width, height, format)?;

    // 主动释放：一次性工具，但让合成器及时回收资源。
    frame.destroy();
    session.destroy();
    buffer.destroy();
    pool.destroy();
    source.destroy();

    Ok(Shot {
        width,
        height,
        pixels,
    })
}

/// 按偏好挑一个我们能解码的格式。不透明格式优先：抓的是屏幕内容，
/// alpha 通道没有意义，X 版本省掉一次无谓的 alpha 处理。
///
/// cosmic-comp 实际提供的是 Xbgr8888 / Abgr8888（不含 Xrgb8888），所以两类都要支持。
fn pick_format(offered: &[wl_shm::Format]) -> Result<wl_shm::Format> {
    use wl_shm::Format::*;
    const PREFERRED: [wl_shm::Format; 4] = [Xbgr8888, Xrgb8888, Abgr8888, Argb8888];
    PREFERRED
        .into_iter()
        .find(|f| offered.contains(f))
        .ok_or_else(|| anyhow::anyhow!("合成器未提供可解码的 shm 格式，它给出的是：{offered:?}"))
}

/// 转成 PNG 要的 RGBA8。
///
/// 注意 wl_shm 的格式名描述的是**主机字节序下 32 位整数**的通道排列，不是内存字节序。
/// 小端机器上二者恰好相反：
///   - `Xrgb8888` = 0xXXRRGGBB → 内存 B,G,R,X → 需要交换 R/B
///   - `Xbgr8888` = 0xXXBBGGRR → 内存 R,G,B,X → 已经就是 RGBA，直接拷
/// 按格式名想当然会拿到蓝红颠倒的图。
fn to_rgba(src: &[u8], width: u32, height: u32, format: wl_shm::Format) -> Result<Vec<u8>> {
    use wl_shm::Format::*;
    let (swap_rb, opaque) = match format {
        Xrgb8888 => (true, true),
        Argb8888 => (true, false),
        Xbgr8888 => (false, true),
        Abgr8888 => (false, false),
        other => bail!("暂不支持的 shm 像素格式：{other:?}"),
    };
    let n = (width * height) as usize;
    let mut out = vec![0u8; n * 4];
    for i in 0..n {
        let s = &src[i * 4..i * 4 + 4];
        let d = &mut out[i * 4..i * 4 + 4];
        if swap_rb {
            d[0] = s[2];
            d[1] = s[1];
            d[2] = s[0];
        } else {
            d[0] = s[0];
            d[1] = s[1];
            d[2] = s[2];
        }
        d[3] = if opaque { 255 } else { s[3] };
    }
    Ok(out)
}

fn memfd_of_size(len: usize) -> Result<std::fs::File> {
    use std::os::fd::FromRawFd;
    let name = std::ffi::CString::new("snapocr-shot")?;
    // MFD_CLOEXEC = 1
    let fd = unsafe { libc_memfd_create(name.as_ptr(), 1) };
    if fd < 0 {
        bail!("memfd_create 失败：{}", std::io::Error::last_os_error());
    }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.set_len(len as u64)?;
    Ok(file)
}

extern "C" {
    #[link_name = "memfd_create"]
    fn libc_memfd_create(name: *const std::ffi::c_char, flags: u32) -> i32;
}

// ---- Dispatch：只关心 session 与 frame 的事件，其余对象无事件或可忽略 ----

impl Dispatch<ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_session_v1::Event;
        match event {
            Event::BufferSize { width, height } => state.buffer_size = Some((width, height)),
            Event::ShmFormat { format } => {
                if let wayland_client::WEnum::Value(f) = format {
                    state.shm_formats.push(f);
                }
            }
            Event::Done => state.session_done = true,
            Event::Stopped => {
                state.frame_failed = Some("session stopped".into());
                state.session_done = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_frame_v1::Event;
        match event {
            Event::Ready => state.frame_ready = true,
            Event::Failed { reason } => {
                state.frame_failed = Some(format!("{reason:?}"));
            }
            _ => {}
        }
    }
}

macro_rules! ignore_events {
    ($($iface:ty),* $(,)?) => {$(
        impl Dispatch<$iface, ()> for State {
            fn event(
                _: &mut Self,
                _: &$iface,
                _: <$iface as wayland_client::Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    )*};
}

ignore_events!(
    ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
    ext_image_capture_source_v1::ExtImageCaptureSourceV1,
    ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1,
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    wl_buffer::WlBuffer,
    wl_output::WlOutput,
);
