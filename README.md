<div align="center">

# Easy Typst Editor

**Edit your Typst output the way you'd edit a PDF in Adobe Acrobat — except every change goes back into clean `.typ` source.**

Open a `main.typ`, see every page exactly as Typst typesets it, click on the text and type.
Ctrl+S writes the change back into the right spot of the right `.typ` file.

🐞 **Found a bug or have an idea? [Open a GitHub issue](https://github.com/qnai-ai/Easy_Typst_Editor/issues/new/choose) — I fix reported issues within 24 hours.**

[![Latest release](https://img.shields.io/github/v/release/qnai-ai/Easy_Typst_Editor?label=download)](https://github.com/qnai-ai/Easy_Typst_Editor/releases/latest)
![Windows · macOS · Linux](https://img.shields.io/badge/platform-Windows%20%C2%B7%20macOS%20%C2%B7%20Linux-0078d4)
![Typst 0.15.1](https://img.shields.io/badge/Typst-0.15.1-239dad)
[![License: MIT](https://img.shields.io/badge/license-MIT-green)](LICENSE)

**English** · [한국어](README.ko.md)

![Easy Typst Editor screenshot](docs/screenshot-en.png)

</div>

## Why?

[Typst](https://typst.app) is a wonderful typesetting system — but the people who need to *fix a typo in the book*
(editors, co-authors, proofreaders, teachers, students) often don't want to touch markup.
Easy Typst Editor lets them work on the **rendered pages**, while the result is still an ordinary,
version-control-friendly Typst project.

It was built to edit real books: a 277-page textbook re-typesets in about half a second per edit.

## Features

- **What you see is the real output.** Typst 0.15.1 is embedded, so pages, fonts and page breaks match `typst compile`.
- **Click and type, like a word processor.** Insert, delete, paste, select by drag / double-click (word) / triple-click (paragraph) /
  Shift+arrows, move by word and paragraph, find and replace. See [Keyboard](#keyboard).
- **Hold Backspace and it keeps going.** Deleting walks straight through inline template markup such as `#lk("a")[word]`,
  removes elements that end up empty, and steps from one text box into the next, so a run of entries can be cleared with the key alone.
  An item written as a function argument (`#stack(check[A], check[B])`) goes away the same way once its text is gone; an item that
  is already empty has no text to click, so press Backspace at the start of the next item or Delete at the end of the previous one.
  A selection may span several text boxes: the text inside each is deleted and the structure is kept.
- **Formatting that writes clean Typst:** Ctrl+B → `*bold*`, Ctrl+I → `_italic_`, Ctrl+U → `#underline[...]` (or your template's own `#u[...]`),
  strikethrough, paragraph alignment (`#align(center)[...]`), Enter → new paragraph, Shift+Enter → `\` line break, Tab, hanging indent.
  Typing over a selection keeps its formatting. Ctrl+B follows what is actually typeset, not only the marks in the source:
  it also turns off bold that comes from `#text(weight: "bold")[...]`, a template function or a `#show` rule.
  Ctrl+U does the same for an underline drawn by a rule (for example `#show emph: it => underline(it.body)`): it removes the
  `_..._` that the rule decorates and keeps the bold if there was one.
- **Multi-file projects.** Edits go to whichever included file the text came from (`content/ch01.typ`, …).
- **Template text is protected.** Page numbers, outlines, headers and other computed text are locked — click them and the editor explains why.
- **Move or delete whole blocks.** Alt+click any element (a placed label, a heading line, a template element) to drag it or delete it.
- **Resize boxes and tables with the mouse.** Click into a box or table and faint handles appear on its edges: drag the right edge
  for the width, the bottom edge for the height, a line between two columns or rows for that column or row. The editor rewrites
  `width:` / `height:` / `columns:` / `rows:` in the source and keeps the unit you wrote (`60mm` stays mm, `80%` stays %).
  A value given as a variable is changed where the variable is set.
- **One template box, not all of them.** Dragging a box that a template function draws changes only the one you dragged: the function
  gets a parameter (`#let key(body, width: 100%) = block(width: width, ..)`) and the call gets the value (`#key(width: 80%)[..]`).
  Hold Shift while dragging to change every place the template is used.
- **Font size and colour.** Toolbar buttons and Ctrl+[ / Ctrl+]. If the selection is the whole content of a `#text(..)[..]`, its
  argument is changed; otherwise the selection is wrapped in `#text(size: ..)[..]` / `#text(fill: ..)[..]`.
- **Images.** Click an image to pick it, drag to resize (the aspect ratio is kept), or replace the file: the new file is copied
  next to the old one and the path in the source is rewritten.
- **Table rows and columns.** Right-click in a table to insert or delete a row or a column. Empty cells can be clicked and typed into.
- **Safe by design.** Every edit is checked against the syntax tree and rolled back if it would break compilation; nothing touches the disk until you save;
  the original is backed up before each save; and it refuses to overwrite a file another program changed in the meantime.
- **Developer view.** Optionally shows the source line you are editing.
- **5 UI languages:** English, 한국어, 日本語, 简体中文, Español (🌐 menu, top-right).

## Download & run

Download the package for your system from **[Releases](https://github.com/qnai-ai/Easy_Typst_Editor/releases/latest)**,
unpack it anywhere, start the editor and choose your project's `main.typ`.
Try [`examples/hello/main.typ`](examples/hello/main.typ) first.

| System | Package | Run | Needs |
|---|---|---|---|
| **Windows 10/11** | `EasyTypstEditor-vX.Y.Z-windows-x64.zip` | `EasyTypstEditor.exe` (keep `WebView2Loader.dll` next to it if the zip has one) | Microsoft Edge WebView2 Runtime (preinstalled on current Windows) |
| **macOS 11+** (Apple Silicon and Intel) | `EasyTypstEditor-vX.Y.Z-macos-universal.zip` | `Easy Typst Editor.app` | nothing else |
| **Linux** (x86-64) | `EasyTypstEditor-vX.Y.Z-linux-x64.tar.gz` | `./EasyTypstEditor` | WebKitGTK 4.1 and GTK 3 |

- **macOS:** the app is not notarized by Apple, so the first start is blocked. Allow it once under
  *System Settings → Privacy & Security → Open Anyway*, or run `xattr -dr com.apple.quarantine "Easy Typst Editor.app"`.
  To open `.typ` files with a double-click, use Finder's *Get Info → Open with → Change All*.
- **Linux:** install the web view if it is missing — Debian/Ubuntu `sudo apt install libwebkit2gtk-4.1-0`,
  Fedora `sudo dnf install webkit2gtk4.1`, Arch `sudo pacman -S webkit2gtk-4.1`. `sh register.sh` adds the editor to the
  application menu and opens `.typ` files with a double-click (`sh register.sh -u` undoes it).
  The file chooser comes from your desktop (XDG desktop portal; `zenity` is used if there is none).
- **Windows:** run `register.ps1` to open `.typ` files with a double-click.

You can also open a file directly: `EasyTypstEditor "path/to/main.typ"`.

macOS and Linux support is new — if something behaves differently there, please [open an issue](https://github.com/qnai-ai/Easy_Typst_Editor/issues/new/choose).

## Keyboard

On **macOS** use ⌘ where the table says Ctrl and ⌥ where it says Alt. Caret movement follows the Mac habit:
⌥←/→ by word, ⌥↑/↓ by paragraph, ⌥+Delete deletes a word, ⌘←/→ line start/end, ⌘↑/↓ document start/end.
F1 shows the keys for the system you are on.

| Keys | Action |
|---|---|
| Click / drag / Shift+click / Shift+arrows | Caret / select |
| Double-click · triple-click · Ctrl+A | Select word · paragraph · the whole text box |
| Backspace · Delete | Delete a character; hold to keep deleting across markup and text boxes |
| Ctrl+Backspace · Ctrl+Delete | Delete a word |
| Ctrl+← · Ctrl+→ · Ctrl+↑ · Ctrl+↓ | Move by word · by paragraph |
| Home · End · Ctrl+Home · Ctrl+End | Line start · end · document start · end |
| Ctrl+X · Ctrl+C · Ctrl+V | Cut · copy · paste (also in the right-click menu) |
| Ctrl+B · Ctrl+I · Ctrl+U · Ctrl+Shift+X | Bold · italic · underline · strikethrough |
| Ctrl+] · Ctrl+[ | Larger · smaller text |
| Ctrl+L · Ctrl+E · Ctrl+R · Ctrl+J | Align left · center · right · back to the template's alignment |
| Enter · Shift+Enter · Tab | New paragraph · line break · tab |
| Ctrl+F · Ctrl+H · F3 · Shift+F3 | Find · replace · next · previous match |
| Ctrl+Z · Ctrl+Y | Undo · redo (consecutive typing or deleting is one step) |
| Ctrl+S · Ctrl+O | Save · open |
| Alt+click | Pick a whole element (then drag, arrow keys, or Delete) |
| Drag a handle (click into a box or table to see them) | Width and height of a box · width of a column · height of a row. Shift: every use of the template |
| Click an image | Pick it: resize with the handles, or replace the file |
| Right-click in a table | Insert or delete a row or a column |
| Ctrl+wheel · Ctrl+Plus · Ctrl+Minus · Ctrl+0 | Zoom in/out · fit width |
| Ctrl+G · Page Up · Page Down | Go to page · scroll a screen |
| F1 | Show all shortcuts |

## How it works

Typst knows which source span produced each glyph. The editor builds an index from every glyph on the page to its
source position, **verifies** it against the source text, and recovers positions that `show` rules lose by matching the
rendered text against the source. Text that cannot be mapped safely is locked rather than guessed.
Each edit is applied to the source in memory, the document is re-typeset incrementally, and the pages are redrawn as SVG.

```
src/world.rs    Typst World: files in memory, written to disk only on save
src/index.rs    glyph ↔ source index (recomputed only for changed pages)
src/recover.rs  verifying / recovering source positions
src/edit.rs     source edits (insert, delete, formatting, paragraphs, indent, alignment, elements) + unit tests
src/session.rs  editing session: caret, selection, movement, find/replace, undo, save
src/lang.rs     UI language (strings in ui/i18n.json)
src/main.rs     window (the system web view: WebView2 / WKWebView / WebKitGTK) + local UI server on 127.0.0.1
ui/index.html   the UI
```

## Build from source

```powershell
# Windows — Rust (MSVC toolchain, or GNU toolchain + MinGW)
powershell -ExecutionPolicy Bypass -File build.ps1   # → dist\EasyTypstEditor.exe
cargo test --release
```

```sh
# Linux — Rust, plus on Debian/Ubuntu: sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev
# macOS — Rust and the Xcode command line tools
sh build.sh              # → dist/EasyTypstEditor   or   dist/Easy Typst Editor.app
UNIVERSAL=1 sh build.sh  # macOS: one app for Apple Silicon and Intel
cargo test --release
```

[`.github/workflows/build.yml`](.github/workflows/build.yml) builds and tests all three systems on every push, and attaches the
packages to the release when a `v*` tag is pushed.

Headless mode for development: `EasyTypstEditor --serve 47615 main.typ`, then open `http://127.0.0.1:47615/`.

Settings, the recent list and the backups made before each save live in `%LOCALAPPDATA%\EasyTypstEditor` (Windows),
`~/Library/Application Support/EasyTypstEditor` (macOS) or `~/.local/share/EasyTypstEditor` (Linux).
Typst packages are looked up where `typst` itself keeps them (`TYPST_PACKAGE_PATH` and `TYPST_PACKAGE_CACHE_PATH` are honoured).

## Known limitations

- Handles appear for boxes the editor can trace back to the source: tables, grids, boxes with a fill or a border, images, and boxes
  written directly in the document. An invisible box created inside a template function cannot be traced, and a size written as an
  expression (`if`, a function call) is not rewritten.
- Rows and columns can be inserted or deleted only in tables whose cells are written one by one in the source: not in tables built
  by code (`..rows.map(..)`), and not when there are merged cells or `table.header(..)` groups.
- The caret in an empty paragraph (right after Enter) is drawn at an estimated position.
- Find works on the text as printed on the page. "Replace all" changes only matches whose source is plain text
  (matches inside template-generated text, or spanning markup, are skipped and counted).
- If the same text appears several times on a page and cannot be told apart by context, it is locked (safer than editing the wrong one).

## Bugs & suggestions — fixed within 24 hours

Please report anything that goes wrong, and anything you wish it did, on **[GitHub Issues](https://github.com/qnai-ai/Easy_Typst_Editor/issues/new/choose)**.
I fix reported issues within 24 hours. A minimal `.typ` file (or a screenshot) that shows the problem makes it much faster.

## Contributing

Pull requests are welcome too — especially:

- **Translations:** all UI strings live in [`ui/i18n.json`](ui/i18n.json). Add your language code to each entry and to `LANGS` in `src/lang.rs`.
- Templates or documents where text gets locked unexpectedly (a minimal `.typ` reproduction helps a lot).
- Reports from macOS and from Linux desktops (Wayland compositors, other distributions).

If this tool saves you time, a ⭐ helps other Typst users find it.

## License

[MIT](LICENSE). Easy Typst Editor is not affiliated with Typst GmbH or Adobe; it embeds the open-source [Typst](https://github.com/typst/typst) compiler (Apache-2.0).
