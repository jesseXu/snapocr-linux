# SnapOCR for Linux — 设计对齐文档

> 对齐日期：2026-09-01 ｜ 状态：第 1 步（snapocr-shot）已完成并实测通过
> 参考实现：macOS 版 `~/dev/vibes/screenshot`（Swift/AppKit，已上架 Mac App Store）

---

## 1. 这是什么

把 macOS 版 SnapOCR 的**使用体验**搬到 Linux 桌面。

明确不是移植架构。macOS 版依赖的 ScreenCaptureKit / Vision / Carbon 热键 / AppKit 在 Linux 全无对等物，逐一找替身只会得到一个别扭的四不像。**目标是终端体验相似，技术选型完全按 Linux 上顺手的方式来。**

首个目标平台：Pop!_OS 24.04 + COSMIC（Wayland）。可移植性边界见 §8。

## 2. 设计意图（继承自 macOS 版）

出发点不变 ——「在保持极简的前提下，比现有方式快一步」：

1. **截图比系统截图少一步**：框选松手就**直接进剪贴板**，拿来即用；要存档再按一次键。
2. **取字跳过「先截图再喂给 OCR」**：框选一个区域，文字直接就绪，不需要存文件、开另一个程序、拖进去。

定位是轻量、快捷键驱动的常驻小工具。不做录屏、图床、美化。

## 3. v1 范围

### 做

- 区域截图 → 剪贴板（+ 存盘）
- 区域取字 → 异步 OCR → 可编辑文本框
- 标注编辑器：钢笔手绘 / 箭头 / 自动编号的数字标记点
- 全局快捷键、开机自启

### 不做（与 macOS 版的关键差异）

- **砍掉全屏就地 OCR**。macOS 版按一个键识别整屏、像文本框一样在屏幕上直接拖选。v1 不做，改为「框选区域 → 文本进文本框」。
- 多显示器：v1 只处理鼠标所在那块屏（与 macOS 版一致）。
- 托盘图标、设置界面：可延后到收尾阶段。

### 为什么砍掉全屏 OCR

这不是省事，是**砍掉了项目最高的技术风险**：

- macOS 版的就地选取建立在 Vision 的 `candidate.boundingBox(for: range)` 之上 —— 对**任意子串**返回精确矩形。Linux 上没有对等能力。
- tesseract 能给逐字框，但中文准确率明显不足；PaddleOCR 中文好，却只给行级四边形，要复刻交互得自己拿 rec 模型的 CTC 输出做列对齐反推每字 x 区间 —— 实打实的自研，且是最容易翻车的一块。
- 砍掉之后，OCR 的输出需求退化为「文本 + 阅读顺序」，`tesseract` apt 装完即可用。

**附带的更大收益**：文本进的是**可编辑**文本框，识别错了用户直接改。macOS 版识别错就是错了只能重来。这让「Linux 上没有 Vision」从硬伤降级为可接受的差异。

## 4. 交互规格

两条流程**前半段完全相同**，分叉只发生在松手之后。模式在框选**之前**由快捷键决定，不在截图后再问。

| 快捷键 | 模式 | 流程 |
| --- | --- | --- |
| `⌃⌥A` | 截图 | 冻结屏幕 → 暗化 → 拖框（实时显示像素尺寸）→ 松手 → **图进剪贴板** → toast |
| `⌃⌥S` | 取字 | 冻结屏幕 → 暗化 → 拖框 → 松手 → 异步 OCR → **文本窗口** |

沿用 macOS 版的按键，肌肉记忆不用改；`⌃⌥S` 从「全屏识别」变为「框选取字」。

**截图 toast**（底部浮出，约 4 秒自动消失）：

- `S` 保存 PNG 到 `~/图片` 或 `~/Pictures`（按 XDG 用户目录）
- `E` 打开标注编辑器
- `Esc` 立即关闭

**取字文本窗口**：

- 文本可编辑（就地修正识别错误）
- 可选取部分复制；一个「全部复制」按钮
- `Esc` 关闭

**标注编辑器**：钢笔 / 箭头 / 数字标记点（自动 1..N 编号），6 色，`⌘Z`→`Ctrl+Z` 撤销，复制或保存时按**原始像素**重绘（非显示分辨率）。

**框选浮层通用**：`Esc` 取消；十字光标；选区外暗化、选区内还原全亮。

## 5. 架构

拆成两块，把唯一有协议风险的部分隔离成一个小工具，写一次之后不再碰：

```
COSMIC 自定义快捷键 (Spawn)
        │
        ├─ snapocr-shot --mode=copy  ─┐
        └─ snapocr-shot --mode=ocr   ─┤   Rust，~500 行
                                      │   冻结抓屏 + 暗化框选浮层 → 输出 PNG
                                      ▼
                              主程序（编排层）
                              ├─ 剪贴板  → wl-copy
                              ├─ toast   → 小浮层窗口
                              ├─ OCR     → tesseract（子进程）
                              ├─ 文本窗口
                              └─ 标注编辑器
```

这个拆法符合 Unix 惯例：协议层的脏活给一个小工具，编排逻辑留在好改的地方。`snapocr-shot` 本身有独立价值 —— COSMIC 生态目前确实缺一个能用的 grim+slurp 替代品。

**抓屏与浮层必须做成后端接口**（trait / 抽象层），v1 只实现 COSMIC 能跑的那条。理由见 §8。

## 6. 环境事实

以下**均为本机实测结论**（Pop!_OS 24.04 + COSMIC，`cosmic-comp`）。凭直觉选型会撞墙，逐条记录以免重复踩。

### 死路（已验证）

| 想当然的做法 | 实际结果 |
| --- | --- |
| 用 `grim` 抓屏 | ❌ `cosmic-comp` **不提供** `zwlr_screencopy_manager_v1`（grep 二进制 0 匹配）；apt 里的 grim 1.4.0 只会这个协议，装了也是废的 |
| `cosmic-screenshot --interactive=false` | ❌ 实测 `Portal request didn't succeed: Other` |
| `org.freedesktop.portal.GlobalShortcuts` 注册热键 | ❌ 本机 portal **无此接口**（已列全部 23 个接口确认） |
| GTK4 + `gtk4-layer-shell` 做浮层 | ❌ apt **未打包** |
| X11 工具（xclip 之外）截图 | ❌ Xwayland 看不见 Wayland 窗口 |

### 可用（已验证）

| 能力 | 手段 |
| --- | --- |
| 抓屏 | `ext_image_copy_capture_manager_v1` + `ext_output_image_capture_source_manager_v1` |
| 浮层 | `zwlr_layer_shell_v1` |
| 剪贴板 | `zwlr_data_control_manager_v1` / `ext_data_control_manager_v1` → `wl-clipboard` 2.2.1 |
| 全局快捷键 | COSMIC 自定义快捷键的 `Spawn("cmd")` 动作 |
| OCR | `tesseract-ocr` 5.3.4 + `tesseract-ocr-chi-sim` / `chi-tra` |
| 托盘 | SNI（`org.kde.StatusNotifierWatcher` 在跑，`cosmic-applet-status-area` 已装） |
| 开机自启 | `~/.config/autostart/*.desktop`（XDG 标准） |
| 区域选择（备选） | `slurp` 1.5.0 可用（只依赖 layer-shell，不依赖 screencopy） |
| GTK3 浮层（备选） | `libgtk-layer-shell0` + `gir1.2-gtklayershell-0.1` 已打包 |
| Qt 浮层（备选） | `layer-shell-qt` / `liblayershellqtinterface5` 已打包 |
| 工具链 | rustc/cargo 1.95、gcc 13.3、python 3.12；crates.io 与 pypi 均可达 |

### 实现 snapocr-shot 时新踩到的坑（全部已实测）

| 坑 | 实情 |
| --- | --- |
| shm 像素格式 | cosmic-comp 只给 `Xbgr8888` / `Abgr8888`，**不给** `Xrgb8888`。只认后者会直接失败 |
| 格式名的含义 | `wl_shm` 格式名描述的是**主机字节序下 32 位整数**的通道排列，不是内存字节序。小端机上二者相反：`Xrgb8888` 内存序是 B,G,R,X 而 `Xbgr8888` 是 R,G,B,X。按名字想当然会得到蓝红颠倒的图，且不会报错 |
| `wl_surface` frame 回调 | **cosmic-comp 上收不到**。用它做渲染节流会在第一帧后永久卡住（屏幕变暗但选区框再不刷新）。改用时间节流 |
| 单块缓冲复用 | 会死锁：合成器在收到新缓冲前不 release 旧的，而客户端又因它被占用而跳过绘制。应每帧从 `SlotPool` 取（池自会复用已释放槽位），靠节流控制提交频率 |
| output 枚举顺序 | **跨运行不稳定**，两次运行 `HDMI-A-1` / `HDMI-A-2` 的下标会互换。不能用下标标识屏幕，一律用名字 |
| output 名字的获取时机 | 必须在枚举时一并取出。抓屏阶段会另开事件队列，之后再按 proxy 反查 `output_state.info()` 会拿不到 |
| `wp_cursor_shape_manager_v1` | 可用，一行拿到十字光标。mac 版为此写了「定时器反复 set」的 hack，这边反而干净 |

### OCR 质量实测（2026-09-01）

用真实 4K 桌面截图跑 `tesseract -l chi_sim+eng`：中文准确（「PopOS Linux 逻辑实现」完全正确），
终端等宽英文基本准确，个别图标/符号被识别成杂字（`米`、`»`）。全屏 4K 约 2.3s，
框选的小区域约 0.4s。

结论：**质量可用**。配合可编辑文本框（识别错了当场改），足以支撑 v1。

### 仍未验证（后续步骤再确认）

- COSMIC 自定义快捷键配置文件的**确切 RON 语法**（`~/.config/cosmic/com.system76.CosmicSettings.Shortcuts/v1/custom`，当前为空目录；`Spawn` 动作与 `custom` 键名已从二进制确认存在）
- portal Screenshot 交互模式的行为（未测，会弹 UI）

## 7. 技术选型

| 组件 | 选型 | 理由 |
| --- | --- | --- |
| `snapocr-shot` | **Rust** + wayland-client | 本机唯一可行路径；单文件静态二进制；启动快（一次性进程，延迟直接影响手感） |
| 主程序 | **Rust**，或 Python + GTK3 | 纯普通窗口，怎么快怎么来 |
| OCR | `tesseract` 子进程 | apt 一条命令；关词典纠正用 `load_system_dawg=0 load_freq_dawg=0`，对应 macOS 版的「语言修正」开关 |
| 剪贴板 | `wl-clipboard` | 一条命令 |
| 快捷键 | 写 COSMIC 配置文件 | 程序可自己写入，用户体验接近原生设置面板 |

**唯一一处「不按方便优先」的地方是抓屏。** 所有省事的路都试过且都不通（§6），而「框选松手即进剪贴板」正是产品的立身之本，所以这里选体验、认下这 ~500 行协议代码。其余全部按 Linux 顺手的方式来。

## 8. 可移植性边界

选型时刻意避开了 COSMIC 私有协议：`ext-image-copy-capture-v1` 是 freedesktop **标准**（`ext_` 前缀，非 `zcosmic_`），`zwlr_layer_shell_v1` 是 wlroots 系事实标准。因此设计天然具备跨桌面潜力。

| 桌面 | 抓屏 | 浮层 | 剪贴板 | 全局热键 | 结论 |
| --- | --- | --- | --- | --- | --- |
| **COSMIC** | ext-image-copy-capture | layer-shell | data-control | COSMIC 配置 | ✅ v1 目标 |
| **wlroots 系**<br>(sway/hyprland/niri) | ext-image-copy-capture 或 wlr-screencopy | layer-shell | data-control | 各自配置文件 | 🟢 加一个 wlr-screencopy 后端即可 |
| **KDE / KWin** | 待确认 | layer-shell ✓ | data-control ✓ | GlobalShortcuts portal ✓ | 🟡 抓屏需在目标机验证 |
| **GNOME** | 仅 portal | ❌ 拒绝 layer-shell | ❌ 无 data-control | GlobalShortcuts portal | 🔴 需单独写一整条 portal 路径 |
| **X11 桌面**<br>(Xfce/Cinnamon/i3) | XGetImage | 普通 override-redirect 窗口 | xclip | XGrabKey | 🟢 另一条路，但最简单 |

结论：

- **抓屏与浮层做成可插拔后端**，v1 只实现 COSMIC 那条。通用性预留在架构里，不在 v1 实现。
- **GNOME 是最大的例外**，它对以上三项协议全部拒绝，一切强制走 portal。要支持得单独投入，不在 v1 考虑。
- **全局快捷键无法抽象掉** —— Wayland 的固有设计使然，每个桌面都得单独适配。这部分应做成「安装步骤」而非运行时能力，为常见桌面各附一个写配置的脚本。

## 9. 落地顺序

1. ~~**`snapocr-shot`** —— 冻结抓屏 + 暗化框选浮层，输出 PNG。~~ **✅ 已完成**
   实测：双屏 3840x2160 + 2560x2880，抓屏各约 300ms；框选、裁剪、十字光标、
   实时尺寸标签均可用；看门狗与 `--outputs` 诊断就位；编译零警告。
2. ~~**截图流程闭环** —— `wl-copy` + 通知 + 存盘。~~ **✅ 已完成**
3. ~~**取字流程** —— tesseract + 文本窗口。~~ **✅ 已完成（待真机验收）**
4. ~~**标注编辑器**~~ **✅ 已完成**
5. ~~**收尾** —— 写 COSMIC 快捷键配置。~~ **✅ 已完成**
   `snapocr install/uninstall/status`。**autostart 与托盘图标最终判定为不需要**：
   全部是一次性进程，快捷键注册在 COSMIC 配置里本身就持久，没有常驻进程要拉起，
   也就没有「找回菜单栏图标」这类需求。这是相对 macOS 版的架构简化。

第 1 步做完即可判定方案成立与否；第 2 步做完就能天天用。

## 10. 相对 macOS 版的体验差异（已知且接受）

| 项 | macOS | Linux v1 |
| --- | --- | --- |
| 全屏就地 OCR 选取 | 有 | **无**（改为框选取字） |
| OCR 准确率 | Vision，较高 | tesseract，较低 —— 但文本可编辑，可手改 |
| 快捷键设置位置 | App 内设置面板 | 写入 COSMIC 配置（程序可代写） |
| 权限 | 需授权屏幕录制（TCC） | 无需授权 |
| 签名 / 沙盒 / 公证 / 商店上架 | 大量工作 | **全部不需要** |

macOS 版中围绕签名、App Sandbox、security-scoped bookmark、`DTXcode` provenance 键、App Store ingestion 的整块复杂度，在 Linux 上直接归零。

## 11. 未决问题

均已定案：

- ~~主程序用 Rust 还是 Python~~ → **Python + GTK4**（协议脏活留在 Rust 的 `snapocr-shot`）。
- ~~保存目录~~ → **跟随 XDG `XDG_PICTURES_DIR`**，不硬编码桌面（Linux 上桌面目录未必存在）。
- ~~是否需要托盘图标与设置界面~~ → **不需要**，见 §9 第 5 步。

新的待办：

- 「语言修正」开关（`ocr.recognize(dictionary=False)` 已支持，但还没有开关暴露给用户）。
- 结果窗口自动全量复制是否符合预期（会冲掉剪贴板原有内容）。
