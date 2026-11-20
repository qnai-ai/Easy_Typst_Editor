<div align="center">

<img src="docs/readme/en/hero.png" alt="Easy Typst Editor — Edit Typst like a PDF. Save clean .typ source." width="860">

<br>

[![Download free](https://img.shields.io/github/v/release/qnai-ai/Easy_Typst_Editor?label=Download%20free&style=for-the-badge&color=1f6feb)](https://github.com/qnai-ai/Easy_Typst_Editor/releases/latest)
&nbsp;
![Windows · macOS · Linux](https://img.shields.io/badge/Windows%20%C2%B7%20macOS%20%C2%B7%20Linux-0b1f44?style=for-the-badge)
&nbsp;
[![MIT](https://img.shields.io/badge/License-MIT-28a745?style=for-the-badge)](LICENSE)

**English** · [한국어](README.ko.md)

</div>

<br>

## 😩 Sound familiar?

> 💬 *“Could you fix one typo on page 132, line 3?”* — your co-author, again.
>
> 💬 *“I opened the .typ file… but I have no idea where to change it.”* — your editor, stuck in front of markup.
>
> 💬 *“Every formula ChatGPT gave me is LaTeX.”* — so you retype them all in Typst.

[Typst](https://typst.app) is a wonderful typesetting system — but **not everyone who needs to fix a document knows Typst.**
**Easy Typst Editor** lets you edit right on the typeset *page*, and keeps the result as clean `.typ` source.

<br>

<div align="center"><img src="docs/readme/en/steps.png" alt="Three steps: open main.typ, click and type, Ctrl+S" width="860"></div>

<br>

## ✨ Five things it does well

<div align="center"><img src="docs/readme/en/point-source.png" alt="POINT 1: edit the page, the source changes cleanly" width="860"></div>

- Click any text on the page and type. Select, delete, paste, find & replace — just like a word processor.
- **Ctrl+B → `*bold*`, Ctrl+I → `_italic_`, Ctrl+U → `#underline[..]`.** The source reads as if a person wrote it.
- Multi-file books are fine: each edit lands in the file and line it came from (`content/ch01.typ` …).

<br>

<div align="center"><img src="docs/readme/en/point-math.png" alt="POINT 2: click an equation, it re-typesets as you type" width="860"></div>

- Click an equation to open its source; **the page re-typesets as you type.** A typo never breaks the page.
- Enter to confirm, Esc to cancel, one undo per equation edit. **Ctrl+M** inserts a new one.
- Equation numbers and labels (`<eq:1>`) stay put, and “Equation 1” references follow along.

<br>

<div align="center"><img src="docs/readme/en/point-latex.png" alt="POINT 3: paste LaTeX from ChatGPT, get Typst math" width="860"></div>

- Understands `$..$`, `$$..$$`, `\(..\)` and `\[..\]` — also when pasted into the equation box.
- **Ctrl+Shift+V** pastes plain text. Prices like `$5 and $6` are not mistaken for math.
- Conversion is done by the open-source [tex2typst](https://github.com/qwinsi/tex2typst) (Apache-2.0), entirely on your PC.

<br>

<div align="center"><img src="docs/readme/en/point-table.png" alt="POINT 4: type into table cells, right-click to add rows and columns" width="860"></div>

- Click a cell and type. Right-click to **insert or delete rows and columns.**
- Click inside a table or box and handles appear: drag to resize **column widths, row heights and boxes**, keeping the units in your source (`mm`, `%`, `fr`).
- Click an image to resize it; “Replace image” swaps the file.

<br>

<div align="center"><img src="docs/readme/en/point-safe.png" alt="POINT 5: hard to break, easy to undo" width="860"></div>

- Every edit is syntax-checked; an edit that would break typesetting is **rolled back automatically.**
- Text your template generates (page numbers, outline, references) is locked, and tells you why when clicked.
- Nothing touches the disk until you save. Each save backs up the original, and it won't overwrite a file another program changed meanwhile.

<br>

## 📚 How people use it — use cases

> Cases 1–3 are **illustrative scenarios** of where the editor fits best; case 4 is a real book being made with it.

<table>
<tr><td width="50%" valign="top">

### 📖 Case 1. A co-author's proofreading pass

**Before** An editor reviewing a 300-page textbook PDF finds 40 typos.
They used to annotate the PDF, send it to the author, who hunted each one down in `.typ` and sent a new PDF back.

**Now** The editor opens `main.typ`, fixes the text on the page, hits Ctrl+S.
Same mistake everywhere? Ctrl+F → “Replace all”.

**Result** **Zero** round trips. The author just reviews the changed lines in git.

</td><td width="50%" valign="top">

### 🧮 Case 2. A math teacher's worksheet

**Before** Problems drafted with ChatGPT come with math like `\frac{a}{b}` — all LaTeX.

**Now** Copy the whole answer and paste it on the page: prose stays prose, math becomes Typst math.
Click any formula you don't like and see it re-typeset as you fix it.

**Result** No retyping formulas. A worksheet takes a few pastes.

</td></tr>
<tr><td width="50%" valign="top">

### 🔬 Case 3. A grad student's lab report

**Before** One more measurement means one more table row, and a coefficient changes in an equation. Deadline: one hour.

**Now** Right-click in the table → “Insert row below”, click the cells, type the values.
Drag a handle to widen a cramped column; click the equation and change just the coefficient.

**Result** Equation numbers and “Equation 2” references stay correct — Typst handles them. The source is still clean Typst.

</td><td width="50%" valign="top">

### 🏷️ Case 4. A real 277-page vocabulary textbook

This editor was **born to make a real book.**
A 277-page Korean vocabulary textbook is being edited with it, and each edit re-typesets in **about half a second.**

Even with a complex template (processed headwords, per-chapter files, an automatic outline),
editable text is editable, and template-generated text is safely locked.

</td></tr>
</table>

<div align="center"><img src="docs/screenshot-ko.png" alt="Editing the real 277-page textbook; the source in the developer view on the right" width="860"><br><sub>Case 4 · the real textbook. Right: the source in the developer view (the line being edited is highlighted).</sub></div>

<br>

## 🆚 Compared with editing the source by hand

| | Editing `.typ` by hand | **Easy Typst Editor** |
|---|---|---|
| Finding the spot | Read the PDF, then search the source | **Click it on the page** |
| Syntax slips | One bracket can break the build | **Breaking edits roll back automatically** |
| Math | Memorise the syntax | **Click, edit, see it typeset · paste LaTeX** |
| Adding a table row | Count cells, type `[..]` | **One right-click** |
| Collaborators | Must learn Typst | **If they can use a word processor, they're set** |
| Output | `.typ` | **The same clean `.typ`** (diff-friendly in git) |

<br>

## ⬇️ Download & run

> ⚠️ **Read the [Disclaimer](#disclaimer) before use.** This program is provided "AS IS" without warranty, and the author is not liable for any loss arising from its use, including damaged or lost manuscripts and anything an AI produces. Always keep backups of your work.

Grab the package for your system from **[Releases](https://github.com/qnai-ai/Easy_Typst_Editor/releases/latest)**, unzip, run, and pick a `main.typ`.
First time? Try [`examples/hello/main.typ`](examples/hello/main.typ).

| System | Package | Run |
|---|---|---|
| 🪟 **Windows 10/11** | `…-windows-x64.zip` | `EasyTypstEditor.exe` |
| 🍎 **macOS 11+** (Apple Silicon · Intel) | `…-macos-universal.zip` | `Easy Typst Editor.app` |
| 🐧 **Linux** (x86-64) | `…-linux-x64.tar.gz` | `./EasyTypstEditor` (needs WebKitGTK 4.1) |

<details>
<summary><b>First run on macOS · Linux</b></summary>

- **macOS:** the app isn't notarized by Apple, so the first launch is blocked. Allow it once in *System Settings → Privacy & Security → Open Anyway*, or run
  `xattr -dr com.apple.quarantine "Easy Typst Editor.app"`.
- **Linux:** install the webview if missing — Debian/Ubuntu `sudo apt install libwebkit2gtk-4.1-0`, Fedora `sudo dnf install webkit2gtk4.1`,
  Arch `sudo pacman -S webkit2gtk-4.1`. `sh register.sh` adds it to the app menu and opens .typ files on double-click.
- **Windows:** run `register.ps1` to open .typ files on double-click.

</details>

<br>

## ❓ FAQ

<details><summary><b>Is it really free?</b></summary>

Yes. It's open source under the MIT license — use, modify and redistribute it at home, at work or at school.
</details>

<details><summary><b>Can I use it without knowing Typst?</b></summary>

Yes. Fixing text, making it bold or adding a table row works like a word processor. Only math uses Typst math syntax — and the symbol buttons and LaTeX paste help with that.
</details>

<details><summary><b>Could it break my source?</b></summary>

Every edit is syntax-checked and rolled back if typesetting breaks. Nothing is written until you save, and each save backs up the original.
Ctrl+Z undoes as far back as you like.
</details>

<details><summary><b>Does my document leave my computer?</b></summary>

Not by default. The Typst engine (0.15.1) is built into the app, so everything happens on your PC. The window talks to the engine only on 127.0.0.1,
and that channel is locked with a token created anew on every launch, so other websites and programs cannot cut in.
The editor itself never connects to the internet. If you use **✨ AI**, however, Claude Code on your computer sends the contents of your source to Anthropic — see the next question.
</details>

<details><summary><b>Is editing with “✨ AI” safe? What leaves my computer?</b></summary>

**✨ AI** is an optional feature that calls **[Claude Code](https://claude.com/claude-code), already installed and signed in on your own computer**, and lets it edit your source.
Without Claude Code the feature is off and the editor sends nothing to the internet.

- **You never enter a key into the editor.** It does not accept, store or transmit any API key or password. Claude Code signs in its own way.
- **How to use it:** press **✨ AI** (Ctrl+K) and write, say, “put two spaces inside both ends of the (A) and (B) boxes on this page; same in the answer key”. Claude Code reads the template and the source and edits the files it needs to.
  The editor tells it where the page in view and your selection are in the source, then typesets again and lists what changed.
- **What is sent:** your instruction and **the contents of the files in the document folder** that Claude Code reads while it works go to Anthropic. Claude decides which files to read, so assume **the whole document folder may be sent**. Usage is charged to your own Claude account.
- **What Claude Code is allowed to do:** it is always started in restricted mode. Inside the document folder it can only **read, search and edit** files; it cannot run commands, reach the web, create files or delete files. Your Claude Code settings, hooks and MCP servers are not loaded.
- **Protecting your source:** Claude Code edits **the saved files directly**. The editor keeps a copy of the files as they were in its backup folder; if the result does not typeset it asks Claude to fix it up to twice, and if it still fails everything is put back.
  Every change is listed and one **Undo** (or Ctrl+Z) reverts it. Still, **“it typesets” does not mean “it is right”** — always check the result yourself.
- **Please do this:** ① back up important work first, or keep it in git. ② Do not use the AI feature on documents from sources you do not trust (text inside a document can trick an AI).
  ③ Do not keep passwords or personal data files in the document folder. ④ Do not use it on material that must not leave your organisation (exam papers, contractually confidential work, …).

More in the [manual, “Edit with AI”](docs/manual.md#edit-with-ai-claude-code) and the [Disclaimer](#disclaimer) below.
</details>

<details><summary><b>Does it look exactly like `typst compile`?</b></summary>

Yes. Same Typst 0.15.1 engine and the same fonts, so page size and page breaks match. “Save as PDF” (Ctrl+P) uses that engine too.
</details>

<details><summary><b>Why can't I edit some text?</b></summary>

Page numbers, the outline and reference numbers are computed by your template — those characters aren't in the source, so they're locked. Clicking them tells you why;
Alt+click picks the whole element to move or delete it. If something is locked that shouldn't be, please [open an issue](https://github.com/qnai-ai/Easy_Typst_Editor/issues/new/choose).
</details>

<details><summary><b>Keyboard shortcuts at a glance</b></summary>

On macOS use ⌘ for Ctrl and ⌥ for Alt. Press F1 in the app for the full list.

| Keys | Action |
|---|---|
| Click · drag · Shift+arrows | Caret · selection |
| Ctrl+B · Ctrl+I · Ctrl+U | Bold · italic · underline |
| Ctrl+Z · Ctrl+Y · Ctrl+S · Ctrl+P | Undo · redo · save · save as PDF |
| Ctrl+F · Ctrl+H | Find · replace |
| Ctrl+K | Edit with AI: instruct Claude Code on your computer (when Claude Code is installed) |
| Click an equation · Ctrl+M | Edit equation · insert equation |
| Ctrl+V · Ctrl+Shift+V | Paste (LaTeX → Typst) · paste as plain text |
| Right-click in a table | Insert / delete rows and columns |
| Alt+click | Pick a whole element (move · delete) |

</details>

<br>

## 🐞 Bugs fixed within 24 hours

Found something odd, or want a feature? Tell us in **[GitHub Issues](https://github.com/qnai-ai/Easy_Typst_Editor/issues/new/choose)** — we aim to fix it within 24 hours.
A small `.typ` file or a screenshot that shows the problem makes it much faster.
If it helps you, a ⭐ helps other Typst users find it.

## 📘 Learn more

- **[Full manual](docs/manual.md)** — every feature, all shortcuts, how it works, building from source, known limitations
- Translations welcome: all UI text lives in one file, [`ui/i18n.json`](ui/i18n.json) (한국어 · English · 日本語 · 简体中文 · Español).

<a id="disclaimer"></a>

## ⚠️ Disclaimer

**By downloading or running this program you agree to the following. If you do not agree, do not use it.**

1. **No warranty.** This is open-source software made by an individual and published free of charge. It is provided **“AS IS”**, **without warranty of any kind**, express or implied, including merchantability, fitness for a particular purpose, freedom from errors, accuracy of results and non-infringement.
2. **No liability.** To the fullest extent permitted by law, the author and contributors are **not liable** for **any direct, indirect, incidental, special or consequential damages** arising from the use of, or inability to use, this program — including damaged or lost manuscripts, wrong typesetting and printing accidents, reprint costs, missed deadlines, business interruption, lost revenue or profit, data disclosure, AI service charges and third-party claims.
3. **Backups are your job.** The backups and undo that the program provides are conveniences, not guarantees. Keep your own backups, or keep your sources under version control such as git.
4. **Checking the result is your job.** Before sending anything to a printer, distributing or publishing it, read the final PDF from beginning to end yourself. Decisions made on the strength of the page shown by this program, or of a “Saved” message, are your own responsibility.
5. **This applies all the more to the AI feature.**
   - **✨ AI** runs, at your instruction, a third-party program that you installed and signed in to yourself (Anthropic's Claude Code). **Claude Code edits the source files on your computer directly.**
   - What an AI produces **can be wrong, can damage your source and can change things you did not ask for.** The editor's safeguards (restricted mode, typesetting check, list of changes, undo, backup) reduce these risks; they do not remove them.
   - Using the AI feature **sends the contents of your source to Anthropic.** Deciding whether that material may be sent to an outside service (confidentiality, personal data, exam papers, other people's copyrighted work, your organisation's rules, contractual duties) and obtaining any permission needed is **entirely your responsibility**.
   - How what you send is handled, what it costs, usage limits, and changes to or discontinuation of the service are governed by Anthropic's terms and privacy policy. The author has no control over them and accepts no responsibility for them.
   - Attacks in which text hidden in a document changes what an AI does (prompt injection) cannot be fully prevented. **Do not use the AI feature on documents whose source you do not trust.**
6. **No security warranty.** Several protective measures are in place, but the program is not warranted to be free of vulnerabilities, and the author is not liable for security incidents connected with its use.
7. **No support obligation.** “Bugs fixed within 24 hours” is a goal the author works towards, not a promise or a guarantee. There is no obligation to fix, maintain or keep compatibility, and features may change or disappear without notice.
8. **No affiliation.** This program is not affiliated with, sponsored by or endorsed by Typst GmbH, Anthropic or Adobe. Typst, Claude and Claude Code are trademarks of their respective owners.

The legally operative terms are in [LICENSE](LICENSE) (MIT); where they differ from the text above, LICENSE prevails.

## License

[MIT](LICENSE). Bundles the open-source [Typst](https://github.com/typst/typst) compiler (Apache-2.0) and [tex2typst](https://github.com/qwinsi/tex2typst) (Apache-2.0, license: [`ui/vendor/tex2typst.LICENSE.txt`](ui/vendor/tex2typst.LICENSE.txt)).
Not affiliated with Typst GmbH or Adobe.
