# SnapOCR

A region screenshot and text-capture tool for **COSMIC** and other modern Wayland
compositors. Select a region and the image is in your clipboard; or select a region
and get its text in an editable window. Includes a small markup editor.

[中文说明](README.zh-CN.md) · [Design notes](DESIGN.md)

## Why this exists

On `cosmic-comp` (Pop!_OS 24.04's COSMIC desktop) the usual Wayland screenshot
tooling does not work: **`grim` speaks only `zwlr_screencopy_manager_v1`, which
cosmic-comp does not implement.** It implements the newer freedesktop standard
`ext-image-copy-capture-v1` instead. The bundled `cosmic-screenshot` only works
through an interactive portal flow.

So `snapocr-shot`, the capture half of this project, is essentially
**`grim` + `slurp` for compositors that speak `ext-image-copy-capture-v1`** —
and it is useful on its own, independently of the OCR and markup parts.

## Features

| Shortcut | What it does |
| --- | --- |
| `Ctrl+Alt+A` | Select a region — **the image goes straight to the clipboard** |
| `Ctrl+Alt+S` | Select a region — OCR the text into an **editable** window |
| `Ctrl+Alt+E` | Select a region — open it in the markup editor |

- **Selection overlay**: the screen freezes and dims, the selection stays bright and
  shows its pixel size live, crosshair cursor, `Esc` cancels.
- **Toast**: after a capture a small overlay offers `S` to save and `E` to annotate.
  It is drawn by the app itself, not by the notification daemon (see
  [Design notes](DESIGN.md) for why). Pure icons and digits — nothing to translate.
- **Markup editor**: freehand pen, arrows, and auto-numbered markers, six colours,
  `Ctrl+Z` to undo. Copy and save re-render at the **original pixel resolution**,
  so a 4K screenshot does not come out blurry.
- **No background process.** Everything is a one-shot invocation; the shortcuts live
  in the compositor's own config. Nothing to autostart, no tray icon.

## Install

```bash
git clone https://github.com/jesseXu/snapocr.git
cd snapocr

./scripts/build-deb.sh                        # needs a Rust toolchain
sudo apt install ./snapocr_0.1.0_amd64.deb    # apt pulls in tesseract, wl-clipboard, GTK4

snapocr install                               # register the shortcuts — no sudo:
                                              # they live in your user config
```

`snapocr doctor` checks that every dependency is present and prints the exact
`apt install` line for anything missing. `sudo apt remove snapocr` uninstalls;
`snapocr uninstall` removes the shortcuts.

### Rebinding the shortcuts

Either edit them natively in **Settings → Keyboard → Keyboard Shortcuts → Custom
Shortcuts** — the three entries appear there by name, because this tool writes
COSMIC's own custom-shortcut config — or re-run install:

```bash
snapocr install --shot "Super+Shift+A" --ocr "Super+Shift+S" --markup "Super+Shift+E"
```

Modifiers are `Ctrl` / `Alt` / `Shift` / `Super` (aliases `Control`, `Option`, `Cmd`,
`Win` also work); key names are xkbcommon keysyms. Existing custom shortcuts of yours
are preserved, and the file is backed up before every write.

## Using `snapocr-shot` on its own

```bash
snapocr-shot out.png       # select a region, write a PNG
snapocr-shot -             # ... to stdout
snapocr-shot --outputs     # print each output's physical/logical size and scale
snapocr-shot --full DIR    # non-interactive full-screen capture, one PNG per output
```

No OCR, no Python, no GTK — a single 1.9 MB binary that only needs a compositor
speaking `ext-image-copy-capture-v1` and `zwlr_layer_shell_v1`.

## Compatibility

Only tested on COSMIC. Both protocols used are standard or de-facto standard rather
than COSMIC-specific, so other compositors are plausible but unverified:

| Desktop | Capture | Overlay | Clipboard | Global shortcuts | Status |
| --- | --- | --- | --- | --- | --- |
| **COSMIC** | ext-image-copy-capture | layer-shell | data-control | COSMIC config | Tested |
| **wlroots** (sway/hyprland/niri) | ext-image-copy-capture or wlr-screencopy | layer-shell | data-control | per-compositor config | Should work; needs a wlr-screencopy backend on older versions |
| **KDE / KWin** | unverified | layer-shell | data-control | GlobalShortcuts portal | Capture needs verifying |
| **GNOME** | portal only | no layer-shell | no data-control | GlobalShortcuts portal | Would need an entirely separate portal path |

Global shortcuts cannot be abstracted away — Wayland has no client-side global
hotkeys by design, so each desktop needs its own registration step.

**This cannot be shipped as a Flatpak.** `cosmic-comp` gates both
`ext-image-copy-capture` and `zwlr_layer_shell_v1` behind `client_not_sandboxed`,
so a sandboxed client cannot see either protocol. That is a property of screen
capture, not a packaging problem.

## Architecture

```
snapocr-shot/   Rust. Frozen capture (ext-image-copy-capture-v1) and the
                selection overlay + toast (zwlr_layer_shell_v1). All the
                protocol-level work lives here.
snapocr/        Python + GTK4. Orchestration: clipboard, OCR, result window,
                markup editor, shortcut registration.
```

The split keeps the one genuinely tricky part quarantined in a small binary that
rarely needs to change, while the parts whose behaviour gets iterated on stay in a
language that is quick to edit.

## Known limitations

- OCR uses tesseract, which is less accurate than macOS's Vision framework.
  The result window is editable precisely because of this — fix a mistake in place
  rather than capturing again.
- A selection cannot span two monitors.
- The UI is English only; there is no localization and none is planned.

## Credits

A Linux port of a macOS menu-bar utility of the same name — the interaction design
is carried over, the implementation is not.

## License

MIT — see [LICENSE](LICENSE).
