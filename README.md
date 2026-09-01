# SnapOCR for Linux

Pop!_OS / COSMIC（Wayland）上的截图与取字小工具。macOS 版 SnapOCR 的
Linux 实现 —— 保持使用体验，技术选型完全按 Linux 上顺手的方式来。

设计与取舍见 [DESIGN.md](DESIGN.md)。

## 功能

| 快捷键 | 做什么 |
| --- | --- |
| `Ctrl+Alt+A` | 框选一块区域 → **图片直接进剪贴板** → 通知里可「保存到图片」或「标注」 |
| `Ctrl+Alt+S` | 框选一块区域 → 识别文字 → **可编辑**的结果窗口，自动全量复制 |

框选浮层：冻结画面 + 整屏压暗，选区还原全亮并实时显示像素尺寸，
十字光标，`Esc` 取消。

标注编辑器：钢笔手绘 / 箭头 / 自动编号的数字标记点，6 色，`Ctrl+Z` 撤销。
复制和保存都按**原始像素**重绘，不会因为窗口缩小显示而变糊。

## 安装

```bash
# 1. 系统依赖
sudo apt install wl-clipboard tesseract-ocr tesseract-ocr-chi-sim tesseract-ocr-chi-tra

# 2. 构建抓屏工具
cargo build --release --manifest-path snapocr-shot/Cargo.toml

# 3. 注册全局快捷键（写入 COSMIC 配置，会自动备份并保留你已有的快捷键）
./bin/snapocr install
```

`./bin/snapocr status` 查看注册状态，`./bin/snapocr uninstall` 撤销。

**不需要开机自启**：全部是一次性进程，快捷键注册在 COSMIC 配置里本身就是
持久的，没有常驻进程要拉起。这是相对 macOS 版（常驻菜单栏）的架构简化。

## 手动使用

```bash
./bin/snapocr shot     # 截图
./bin/snapocr ocr      # 取字
```

## 组成

```
snapocr-shot/     Rust。冻结抓屏(ext-image-copy-capture-v1)+ 框选浮层
                  (zwlr_layer_shell_v1),输出 PNG。协议层的脏活都在这里。
snapocr/          Python + GTK4。编排层:剪贴板、通知、OCR、结果窗、标注。
bin/snapocr       启动器。绑快捷键时指向它。
```

`snapocr-shot` 也可以单独用 —— 它本质上是「给 cosmic-comp 用的 grim+slurp」，
而这两个工具在 COSMIC 上都不可用（cosmic-comp 不提供 `wlr-screencopy`）：

```bash
./target/release/snapocr-shot out.png    # 框选并保存
./target/release/snapocr-shot -          # 输出到 stdout
./target/release/snapocr-shot --outputs  # 打印各屏物理/逻辑尺寸与缩放
```

## 已知限制

- 仅在 COSMIC 上验证过。抓屏与浮层用的都是标准/准标准协议，理论上覆盖
  wlroots 系与 KDE，但未实测；GNOME 需要另写一整条 portal 路径。见 DESIGN.md §8。
- OCR 用 tesseract，准确率不及 macOS 的 Vision。结果窗口可编辑正是为此。
- 选区不能跨屏。
