# SnapOCR for Linux

Pop!_OS / COSMIC（Wayland）上的截图与取字小工具。macOS 版 SnapOCR 的
Linux 实现 —— 保持使用体验，技术选型完全按 Linux 上顺手的方式来。

设计与取舍见 [DESIGN.md](DESIGN.md)。

## 安装

```bash
# 从源码打包（需要 Rust 工具链）
./scripts/build-deb.sh

# 安装。apt 会自动装齐 tesseract、wl-clipboard、GTK4 等所有依赖
sudo apt install ./snapocr_0.1.0_amd64.deb

# 注册全局快捷键（用你自己的身份，不要加 sudo —— 快捷键写在你的用户配置里）
snapocr install
```

装完 `snapocr` 就在 PATH 上，与源码目录再无关系，源码删掉也不影响。

`snapocr doctor` 检查依赖是否齐全；`sudo apt remove snapocr` 卸载
（快捷键需另外用 `snapocr uninstall` 清掉，因为它在用户配置里）。

## 用法

| 快捷键 | 做什么 |
| --- | --- |
| `Ctrl+Alt+A` | 框选 → **图片直接进剪贴板** → 通知里还可「保存到图片」或「标注」 |
| `Ctrl+Alt+S` | 框选 → 识别文字 → **可编辑**的结果窗口，并自动全量复制 |
| `Ctrl+Alt+E` | 框选 → **直接进标注编辑器** |

框选浮层：冻结画面 + 整屏压暗，选区还原全亮并实时显示像素尺寸，
十字光标，`Esc` 取消。

截图后底部会浮出一条提示：剪贴板对勾图标 + 选区尺寸，下面是 `[S]` 存盘、
`[E]` 标注两个键帽。4 秒自动消失，按其它任意键也会立刻消掉它。

**界面一律英文，不做多语言。** toast 更进一步做成纯图标 + 数字，连英文都不需要。

### 标注编辑器

`Ctrl+Alt+E` 框选后直接进入；也可以从截图通知的「标注」按钮进入。
错过了通知也不要紧：

```bash
snapocr markup                 # 重新框选一块来标注
snapocr markup --clipboard     # 标注刚才复制进剪贴板的那张图
snapocr markup ~/图片/a.png     # 标注已有文件
```

窗口上方一排是工具和颜色，三个工具按钮用的是图标：

- **波浪线** = 钢笔 —— 按住拖动，自由手绘
- **斜箭头** = 箭头 —— 从起点拖到终点，自动画出箭头头部
- **红色 ① ** = 标记点 —— 单击放置一个红底白字的圆点，自动编号 1、2、3…
- 六个色块切换颜色（标记点固定红色）

图标是用编辑器自己的绘制函数画的 —— 图标就是该工具的真实笔迹。
不走图标主题，因为主题里缺了对应图标时按钮会变成空白。

| 按键 | 作用 |
| --- | --- |
| `Ctrl+Z` | 撤销上一笔 |
| `Ctrl+C` | 复制到剪贴板并关闭 |
| `Ctrl+S` | 保存到图片目录并关闭 |
| `Esc` | 放弃并关闭 |

复制和保存都按**原始像素**重绘，不会因为窗口是缩小显示的而变糊。

### 改快捷键

两种方式，任选：

**在系统设置里改**（推荐）——「设置 → 键盘 → 键盘快捷键 → 自定义快捷键」，
这三条会以「SnapOCR 截图」这样的名字列在那里，直接改键位即可。我们写入的
就是 COSMIC 自己的自定义快捷键配置，所以它的原生界面完全认得。

**用命令行重装**：

```bash
snapocr install --shot "Super+Shift+A" --ocr "Super+Shift+S" --markup "Super+Shift+E"
```

修饰键可写 `Ctrl` / `Alt` / `Shift` / `Super`（也认 `Control`、`Option`、`Cmd`、
`Win` 这些别名）；键名用 xkbcommon 的名字，单个字母直接写，具名键如
`F1`、`Print`、`Escape`。`snapocr status` 看当前注册情况。

写入前会自动备份，且只增删自己那几行，你已有的自定义快捷键不受影响。

## 组成

```
snapocr-shot/     Rust。冻结抓屏(ext-image-copy-capture-v1)+ 框选浮层
                  (zwlr_layer_shell_v1),输出 PNG。协议层的脏活都在这里。
snapocr/          Python + GTK4。编排层:剪贴板、通知、OCR、结果窗、标注。
packaging/        .desktop 文件
scripts/          打包脚本
```

`snapocr-shot` 也可以单独用 —— 它本质上是「给 cosmic-comp 的 grim+slurp」，
而这两个工具在 COSMIC 上都不可用（cosmic-comp 不提供 `wlr-screencopy`）：

```bash
snapocr-shot out.png      # 框选并保存
snapocr-shot -            # 输出到 stdout
snapocr-shot --outputs    # 打印各屏物理/逻辑尺寸与缩放
```

## 不需要开机自启

全部是一次性进程，快捷键注册在 COSMIC 配置里本身就是持久的，没有常驻
进程要拉起。这是相对 macOS 版（常驻菜单栏）的架构简化，也因此不需要
托盘图标。

## 已知限制

- 仅在 COSMIC 上验证过。抓屏与浮层用的都是标准/准标准协议，理论上覆盖
  wlroots 系与 KDE，但未实测；GNOME 需要另写一整条 portal 路径。见 DESIGN.md §8。
- OCR 用 tesseract，准确率不及 macOS 的 Vision。结果窗口可编辑正是为此。
- 选区不能跨屏。
