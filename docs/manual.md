# Easy Typst Editor — full manual

[← Back to the README](../README.md) · [한국어 설명서](manual.ko.md)

## Install

Download the package for your system from **[Releases](https://github.com/qnai-ai/Easy_Typst_Editor/releases/latest)**,
unpack it anywhere, start the editor and choose your project's `main.typ`.
Try [`examples/hello/main.typ`](../examples/hello/main.typ) first.

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

## Everything it does

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
- **Equations.** Click an equation and its Typst source opens in a small box next to it; the page is re-typeset as you type,
  a syntax error is shown in the box instead of breaking the page, and a checkbox switches between inline `$x$` and block `$ x $`.
  Enter keeps the change, Esc throws it away, and the whole edit is one undo step. Ctrl+M (or the ∑ button) inserts a new equation
  at the caret; buttons insert common pieces (`sum_(i=1)^n`, `sqrt()`, `alpha`, `<=` …). Labels such as `<eq:1>` and numbering stay as they are.
- **Paste LaTeX math.** Paste text with LaTeX math (`$\frac{a}{b}$`, `$$..$$`, `\(..\)`, `\[..\]`, e.g. from ChatGPT or a paper) and the
  equations arrive as Typst equations; pasting LaTeX into the equation box converts it too. Ctrl+Shift+V pastes the text unchanged.
  The conversion uses [tex2typst](https://github.com/qwinsi/tex2typst) (Apache-2.0, bundled; see `ui/vendor/tex2typst.LICENSE.txt`).
- **Safe by design.** Every edit is checked against the syntax tree and rolled back if it would break compilation; nothing touches the disk until you save;
  the original is backed up before each save; and it refuses to overwrite a file another program changed in the meantime.
- **Export PDF.** One click (Ctrl+P) writes the pages as you see them, unsaved edits included, using the same Typst engine as `typst compile`.
- **Developer view.** Optionally shows the source line you are editing.
- **5 UI languages:** English, 한국어, 日本語, 简体中文, Español (🌐 menu, top-right).

## Edit with AI (Claude Code)

Press **✨ AI** (Ctrl+K), type an instruction and press Enter (Shift+Enter for a new line). **Claude Code**, installed and signed in on your own computer, edits the source as instructed.
You never enter a key or password into the editor. Without Claude Code the feature is off.

- **Write the instruction as you would to Claude Code.** “On this page, put two spaces inside both ends of the (A) and (B) boxes of question 94. Same in the answer key. Also line up the hanging indent of the choices.”
  Claude Code reads the template's function definitions and the source, and follows the instruction to other pages and other files.
- **What the editor tells it.** The main file, the page in view (which page it is, and the source file and lines its text comes from), and the selected text or the cursor position. So you can write “this page”, “here” or “the selection”.
- **Progress.** The panel shows what is happening (“Claude is reading: tpl.typ”) and the elapsed time. It usually takes tens of seconds to a few minutes. While it runs the button becomes **Cancel**; cancelling puts back everything changed so far.
- **Result.** The editor typesets again and shows Claude's explanation and the list of changes (file · old → new). Look at the page, then press **Keep** or **Undo**.
  If the result does not typeset, Claude is given the error and asked to fix it, up to twice; if it still fails, everything is put back.

Good to know:

- Claude Code edits **the saved files directly**. That is why you are asked to save (Ctrl+S) first if you have unsaved edits. What the AI changed is already in the files; it needs no separate save.
- **Undo** in the panel also puts the files back. If you undo with Ctrl+Z instead, save with Ctrl+S so that the files go back too.
- The files as they were are kept in the data folder under `backup/<document>/<date-time>-ai`.
- You cannot edit while the AI is working (so that the two sets of edits do not collide).
- Text in data files that the template reads (yaml and the like) can be edited too. Such files do not go back with Ctrl+Z, only with **Undo** in the panel.
- It cannot be asked to create or delete files, or to run commands.

### Once: get Claude Code ready

1. Install [Claude Code](https://claude.com/claude-code).
2. Run `claude` once in a terminal and sign in (a subscription account or an API billing account).
3. Press **✨ AI** in the editor. A notice asks you to confirm the first time.

The editor looks for Claude Code in its standard install locations (`~/.local/bin` and so on) and on PATH.

### Safety notes

| Question | Answer |
|---|---|
| Do I enter a key into the editor? | No. The editor does not accept, store or transmit any API key or password. Claude Code handles authentication. The editor itself never connects to the internet. |
| What leaves my computer? | Your instruction, the location details the editor adds (file names, line numbers, the selected text), and **the contents of the files in the document folder** that Claude Code reads while working go to Anthropic. Claude decides which files to read, so assume the whole document folder may be sent. If you do not use the AI feature, nothing is sent. |
| What is the document folder? | The folder that holds `main.typ` (or a parent folder, if typesetting needs files outside it) and everything below it. Do not keep passwords or personal data files there. |
| What can Claude Code do? | The editor always starts Claude Code in **restricted mode** (`--restricted`). Its only tools are reading, searching and editing (`Read`, `Glob`, `Grep`, `Edit`): no command execution, no web access, no creating or deleting files. The file tools are confined to the document folder. Your Claude Code settings, hooks and MCP servers are not loaded, and no conversation history is kept. |
| How is the instruction passed? | Through Claude Code's standard input, without a shell. Nothing you type in the instruction is executed as a command. |
| Can the AI damage my source? | Yes. So the editor ① keeps a copy of the files as they were in the backup folder, ② typesets again and puts everything back if that fails, ③ lists every change, and ④ offers **Undo** and Ctrl+Z. But “it typesets” does not mean “it is right”. Back up important work separately or keep it in git, and always check the result. |
| Can text inside a document trick the AI? | Claude is told that text in the files is data, not instructions, but this cannot be fully prevented. Since there is no command execution and no web access, the worst case is that files inside the document folder are edited wrongly. Even so, **do not use the AI feature on documents whose source you do not trust.** |
| Can another website trigger this behind my back? | No. The channel between window and engine (127.0.0.1) is locked with a token created anew on every launch, and the engine refuses requests from any other origin (Origin / Host checks) as well as being embedded in another site's frame. |
| What does it cost? | It is charged to your own Claude account (subscription limits or API billing, depending on how Claude Code is signed in). Asking for wide-ranging edits in a large document uses a lot. |
| What does Anthropic do with the text? | The terms and privacy policy that apply to your Anthropic account, and your account's settings, govern that. Do not use the feature on material that must not leave your organisation. |

If you entered a Gemini API key in 0.7.0: that method is gone and this version does not read the stored key. On Windows delete `gemini-api-key.EasyTypstEditor` in Credential Manager,
on macOS delete the `EasyTypstEditor` item in Keychain Access, and if you no longer need the key, delete it in Google AI Studio as well.

The author accepts no responsibility for what the AI produces or for any resulting loss. Please read the [Disclaimer](../README.md#disclaimer).

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
| Ctrl+K | Edit with AI: instruct Claude Code on your computer (see [Edit with AI](#edit-with-ai-claude-code)) |
| Ctrl+Z · Ctrl+Y | Undo · redo (consecutive typing or deleting is one step) |
| Ctrl+S · Ctrl+O · Ctrl+P | Save · open · export PDF |
| Alt+click | Pick a whole element (then drag, arrow keys, or Delete) |
| Drag a handle (click into a box or table to see them) | Width and height of a box · width of a column · height of a row. Shift: every use of the template |
| Click an image | Pick it: resize with the handles, or replace the file |
| Right-click in a table | Insert or delete a row or a column |
| Click an equation · Ctrl+M | Edit that equation · insert a new one (Enter: done, Esc: discard) |
| Ctrl+V · Ctrl+Shift+V | Paste (LaTeX math becomes Typst math) · paste unchanged |
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
src/main.rs     window (the system web view: WebView2 / WKWebView / WebKitGTK) + local UI server on 127.0.0.1 (locked with a per-launch token)
src/ai.rs       Edit with AI: calls Claude Code on this computer in restricted mode (the editor handles no keys)
ui/index.html   the UI
```

## Build from source

```powershell
# Windows — Rust (MSVC toolchain, or GNU toolchain + MinGW)
powershell -ExecutionPolicy Bypass -File build.ps1   # → dist\EasyTypstEditor.exe
cargo test --release
```

```sh
# Linux — Rust, plus on Debian/Ubuntu: sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libdbus-1-dev pkg-config
# macOS — Rust and the Xcode command line tools
sh build.sh              # → dist/EasyTypstEditor   or   dist/Easy Typst Editor.app
UNIVERSAL=1 sh build.sh  # macOS: one app for Apple Silicon and Intel
cargo test --release
```

[`.github/workflows/build.yml`](../.github/workflows/build.yml) builds and tests all three systems on every push, and attaches the
packages to the release when a `v*` tag is pushed.

Headless mode for development: `EasyTypstEditor --serve 47615 main.typ`, then open `http://127.0.0.1:47615/`.
To open a document at a given page: `EasyTypstEditor --page 120 main.typ` (the page's position in the PDF, counting from 1).

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
