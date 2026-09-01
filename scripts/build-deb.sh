#!/bin/bash
set -euo pipefail

# 打一个 .deb。这样 apt 会自动装齐依赖、程序进 /usr/bin、不依赖源码目录，
# 也能用 apt remove 干净卸载 —— 在 Debian 系上这是最省事的分发方式。

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VERSION="${VERSION:-0.1.0}"
ARCH="$(dpkg --print-architecture)"
STAGE="$ROOT/build/deb"
OUT="$ROOT/snapocr_${VERSION}_${ARCH}.deb"

echo "==> 构建 snapocr-shot"
cargo build --release --manifest-path snapocr-shot/Cargo.toml

echo "==> 组装 $STAGE"
rm -rf "$STAGE"
mkdir -p "$STAGE/DEBIAN" \
         "$STAGE/usr/bin" \
         "$STAGE/usr/lib/python3/dist-packages/snapocr" \
         "$STAGE/usr/share/applications" \
         "$STAGE/usr/share/metainfo" \
         "$STAGE/usr/share/doc/snapocr"

install -m 755 target/release/snapocr-shot "$STAGE/usr/bin/snapocr-shot"
install -m 644 snapocr/*.py "$STAGE/usr/lib/python3/dist-packages/snapocr/"
install -m 644 packaging/*.desktop "$STAGE/usr/share/applications/"
install -m 644 packaging/*.metainfo.xml "$STAGE/usr/share/metainfo/"
install -m 644 README.md DESIGN.md LICENSE "$STAGE/usr/share/doc/snapocr/"

# 装好之后包已在 dist-packages，启动器不需要再设 PYTHONPATH。
cat > "$STAGE/usr/bin/snapocr" <<'LAUNCHER'
#!/bin/sh
exec python3 -m snapocr "$@"
LAUNCHER
chmod 755 "$STAGE/usr/bin/snapocr"

cat > "$STAGE/DEBIAN/control" <<CONTROL
Package: snapocr
Version: $VERSION
Section: utils
Priority: optional
Architecture: $ARCH
Depends: python3 (>= 3.10), python3-gi, gir1.2-gtk-4.0, python3-cairo,
 wl-clipboard, tesseract-ocr, tesseract-ocr-eng, tesseract-ocr-chi-sim,
 tesseract-ocr-chi-tra
Maintainer: Jesse Xu <888468+jesseXu@users.noreply.github.com>
Description: 截图与取字小工具（COSMIC / Wayland）
 框选一块区域，图片直接进剪贴板；或框选一块区域识别其中的文字，
 结果落在一个可编辑的窗口里。附带钢笔、箭头、数字标记点的标注编辑器。
 .
 抓屏走 freedesktop 标准的 ext-image-copy-capture-v1 协议，因此在
 cosmic-comp 这类不提供 wlr-screencopy 的合成器上同样可用（grim 不行）。
CONTROL

# 安装后提示注册快捷键。不自动写用户配置：那是用户的桌面设置，
# 而且 postinst 以 root 运行，写不到正确的用户目录。
cat > "$STAGE/DEBIAN/postinst" <<'POSTINST'
#!/bin/sh
set -e
if [ "$1" = "configure" ]; then
    echo ""
    echo "SnapOCR 已安装。注册全局快捷键（以你自己的身份运行，不要用 sudo）："
    echo "    snapocr install"
    echo ""
fi
exit 0
POSTINST
chmod 755 "$STAGE/DEBIAN/postinst"

cat > "$STAGE/DEBIAN/prerm" <<'PRERM'
#!/bin/sh
set -e
# 快捷键写在用户目录，root 的 prerm 清不掉，只能提示。
if [ "$1" = "remove" ]; then
    echo "如需一并移除快捷键，请以你自己的身份运行：snapocr uninstall"
fi
exit 0
PRERM
chmod 755 "$STAGE/DEBIAN/prerm"

echo "==> 打包"
fakeroot dpkg-deb --build "$STAGE" "$OUT" >/dev/null
echo "==> 完成：$OUT"
dpkg-deb --info "$OUT" | sed -n '/Package:/,/^$/p' | head -12
