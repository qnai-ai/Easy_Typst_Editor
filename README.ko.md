<div align="center">

<img src="docs/readme/ko/hero.png" alt="쉬운 지면 편집기 — PDF 고치듯 Typst를 고친다. 저장은 깨끗한 .typ 원고로." width="860">

<br>

[![지금 무료로 내려받기](https://img.shields.io/github/v/release/qnai-ai/Easy_Typst_Editor?label=%EB%AC%B4%EB%A3%8C%20%EB%82%B4%EB%A0%A4%EB%B0%9B%EA%B8%B0&style=for-the-badge&color=1f6feb)](https://github.com/qnai-ai/Easy_Typst_Editor/releases/latest)
&nbsp;
![Windows · macOS · Linux](https://img.shields.io/badge/Windows%20%C2%B7%20macOS%20%C2%B7%20Linux-0b1f44?style=for-the-badge)
&nbsp;
[![MIT](https://img.shields.io/badge/%EB%9D%BC%EC%9D%B4%EC%84%A0%EC%8A%A4-MIT-28a745?style=for-the-badge)](LICENSE)

[English](README.md) · **한국어**

</div>

<br>

## 😩 이런 적, 있으시죠?

> 💬 *“132쪽 셋째 줄에 오타 하나만 고쳐 주세요.”* — 공동 저자에게 또 메시지가 왔다.
>
> 💬 *“.typ 파일은 열어 봤는데… 어디를 고쳐야 할지 모르겠어요.”* — 편집자는 마크업 앞에서 멈췄다.
>
> 💬 *“ChatGPT가 준 수식이 전부 LaTeX예요.”* — 하나하나 Typst로 다시 쳤다.

[Typst](https://typst.app)는 훌륭한 조판 시스템이지만, **고치는 사람이 모두 Typst를 알지는 않습니다.**
**쉬운 지면 편집기**는 조판된 *지면* 위에서 바로 고치게 해 주고, 결과는 그대로 깨끗한 `.typ` 원고로 남깁니다.

<br>

<div align="center"><img src="docs/readme/ko/steps.png" alt="딱 세 단계: main.typ 열기, 누르고 고치기, Ctrl+S" width="860"></div>

<br>

## ✨ 핵심 기능 5가지

<div align="center"><img src="docs/readme/ko/point-source.png" alt="POINT 1: 지면을 고치면 원고가 깔끔하게 바뀝니다" width="860"></div>

- 지면의 글자를 누르고 그대로 치세요. 고르기, 지우기, 붙여넣기, 찾기·바꾸기까지 워드와 같습니다.
- **Ctrl+B → `*굵게*`, Ctrl+I → `_기울임_`, Ctrl+U → `#underline[..]`.** 사람이 쓴 것 같은 Typst가 남습니다.
- 여러 파일로 나뉜 책도 OK. 고친 글은 그 글이 나온 파일(`content/ch01.typ` …)의 그 줄로 들어갑니다.

<br>

<div align="center"><img src="docs/readme/ko/point-math.png" alt="POINT 2: 수식도 누르면 고치는 대로 조판됩니다" width="860"></div>

- 수식을 누르면 원문 창이 뜨고, **치는 대로 지면이 바로 다시 조판**됩니다. 문법이 틀려도 지면은 깨지지 않아요.
- Enter는 확인, Esc는 취소, 한 수식을 고친 것은 되돌리기 한 번. 새 수식은 **Ctrl+M**.
- 수식 번호와 꼬리표(`<eq:1>`)는 그대로. 본문의 “식 1” 참조도 알아서 따라갑니다.

<br>

<div align="center"><img src="docs/readme/ko/point-latex.png" alt="POINT 3: ChatGPT의 LaTeX 수식, 붙여 넣으면 Typst 수식으로" width="860"></div>

- `$..$`, `$$..$$`, `\(..\)`, `\[..\]` 모두 알아봅니다. 수식 창에 붙여 넣어도 바꿔 줍니다.
- 그냥 글로 넣고 싶으면 **Ctrl+Shift+V**. `$5 and $6` 같은 금액은 수식으로 오해하지 않습니다.
- 변환은 오픈 소스 [tex2typst](https://github.com/qwinsi/tex2typst)(Apache-2.0)가 맡습니다. 인터넷 연결 없이 PC 안에서 처리합니다.

<br>

<div align="center"><img src="docs/readme/ko/point-table.png" alt="POINT 4: 표는 칸을 눌러 쓰고, 오른쪽 클릭으로 줄·칸 추가" width="860"></div>

- 칸을 누르고 바로 입력. 오른쪽 클릭으로 **줄·칸 넣기와 지우기**.
- 표나 상자 안을 누르면 가장자리에 손잡이가 생깁니다. 끌면 **칸 너비·줄 높이·상자 크기**가 바뀌고, 원고에 적힌 단위(`mm`, `%`, `fr`)를 지킵니다.
- 그림은 눌러서 크기 조절, 「그림 바꾸기」로 파일 교체.

<br>

<div align="center"><img src="docs/readme/ko/point-safe.png" alt="POINT 5: 망가뜨리기는 어렵고 되돌리기는 쉽게" width="860"></div>

- 고칠 때마다 구문을 검사해, 조판이 깨질 고침은 **자동으로 되돌립니다.**
- 쪽 번호·차례·참조처럼 템플릿이 만드는 글은 잠기고, 누르면 이유를 알려 줍니다.
- 저장 전까지 디스크는 그대로. 저장할 때마다 원본을 따로 백업하고, 다른 프로그램이 그새 파일을 바꿨으면 덮어쓰지 않습니다.

<br>

## 📚 이렇게 씁니다 — 상세 사례

> 아래 1~3은 이 편집기가 잘 맞는 **대표적인 상황을 예로 든 것**이고, 4는 실제로 만들고 있는 책입니다.

<table>
<tr><td width="50%" valign="top">

### 📖 사례 1. 교재 공동 저자의 오탈자 수정

**상황** 출판사 편집자가 300쪽 교재 PDF를 검토하다 오탈자 40곳을 찾았습니다.
예전엔 PDF에 메모를 달아 저자에게 보내고, 저자가 `.typ`에서 하나하나 찾아 고친 뒤 다시 PDF를 만들어 보냈습니다.

**이렇게** 편집자가 `main.typ`를 열고, 지면에서 바로 고치고, Ctrl+S.
Ctrl+F로 같은 실수를 찾아 「모두 바꾸기」.

**결과** 저자와 주고받는 왕복이 **0번**. 저자는 git에서 바뀐 줄만 확인하면 끝.

</td><td width="50%" valign="top">

### 🧮 사례 2. 수학 선생님의 학습지

**상황** ChatGPT로 문제를 만들면 수식이 전부 `\frac{a}{b}` 같은 LaTeX로 나옵니다.

**이렇게** 답을 통째로 복사해 지면에 붙여 넣으면 글은 글대로, 수식은 Typst 수식으로 들어갑니다.
마음에 안 드는 수식은 눌러서 고치면 바로 조판된 모습이 보입니다.

**결과** 수식을 다시 칠 필요가 없습니다. 학습지 한 장이 붙여넣기 몇 번으로 끝.

</td></tr>
<tr><td width="50%" valign="top">

### 🔬 사례 3. 대학원생의 실험 보고서

**상황** 측정을 한 번 더 해서 표에 줄을 늘리고, 식 하나의 계수를 바꿔야 합니다. 마감은 한 시간 뒤.

**이렇게** 표 안에서 오른쪽 클릭 → 「아래에 줄 넣기」, 칸을 눌러 값 입력.
좁은 칸은 손잡이를 끌어 넓히고, 식은 눌러서 계수만 고칩니다.

**결과** 식 번호와 “식 2” 참조는 Typst가 그대로 맞춰 줍니다. 원고는 여전히 깔끔한 Typst.

</td><td width="50%" valign="top">

### 🏷️ 사례 4. 실제로 만들고 있는 277쪽 어휘 교재

이 편집기는 **실제 책을 만들려고** 태어났습니다.
277쪽짜리 어휘 교재를 이 편집기로 고치고 있고, 한 번 고칠 때마다 **0.5초 안팎**이면 다시 조판됩니다.

복잡한 템플릿(표제어 가공, 장별 파일, 자동 차례)이 있어도
고칠 수 있는 글은 고치고, 템플릿이 만든 글은 안전하게 잠급니다.

</td></tr>
</table>

<div align="center"><img src="docs/screenshot-ko.png" alt="277쪽 실제 교재를 편집하는 화면. 오른쪽은 개발자용 보기로 본 원고" width="860"><br><sub>사례 4 · 실제 교재 화면. 오른쪽은 「개발자용 보기」로 연 원고(고치는 줄이 노랗게 표시됩니다).</sub></div>

<br>

## 🆚 원고를 직접 고칠 때와 비교하면

| | `.typ` 원고를 직접 고칠 때 | **쉬운 지면 편집기** |
|---|---|---|
| 고칠 곳 찾기 | PDF에서 보고, 원고에서 다시 검색 | **지면에서 바로 누르기** |
| 문법 실수 | 괄호 하나로 조판이 깨질 수 있음 | **깨질 고침은 자동으로 되돌림** |
| 수식 | 문법을 외워서 입력 | **누르고 고치면 바로 조판 · LaTeX 붙여넣기** |
| 표 줄·칸 추가 | 칸 수를 세어 `[..]` 입력 | **오른쪽 클릭 한 번** |
| 함께 일하는 사람 | Typst를 배워야 함 | **워드를 쓸 줄 알면 충분** |
| 결과물 | `.typ` | **똑같이 깨끗한 `.typ`** (git으로 비교 가능) |

<br>

## ⬇️ 내려받기 · 실행

> ⚠️ **쓰기 전에 [면책사항](#disclaimer)을 읽어 주세요.** 이 프로그램은 보증 없이 "있는 그대로" 제공되며, 원고의 손상·유실과 AI가 낸 결과를 포함해 사용으로 생긴 어떤 손해도 만든 사람은 책임지지 않습니다. 원고는 꼭 백업해 두세요.

**[Releases](https://github.com/qnai-ai/Easy_Typst_Editor/releases/latest)** 에서 내 시스템용 꾸러미를 받아 풀고 실행한 뒤 `main.typ`를 고르세요.
처음이라면 [`examples/hello/main.typ`](examples/hello/main.typ)로 시작해 보세요.

| 시스템 | 꾸러미 | 실행 |
|---|---|---|
| 🪟 **Windows 10/11** | `…-windows-x64.zip` | `EasyTypstEditor.exe` |
| 🍎 **macOS 11 이상** (Apple Silicon · Intel) | `…-macos-universal.zip` | `Easy Typst Editor.app` |
| 🐧 **Linux** (x86-64) | `…-linux-x64.tar.gz` | `./EasyTypstEditor` (WebKitGTK 4.1 필요) |

<details>
<summary><b>macOS · Linux에서 처음 실행할 때</b></summary>

- **macOS:** Apple의 공증을 받지 않은 앱이라 처음 실행이 막힙니다. *시스템 설정 → 개인정보 보호 및 보안 → 그래도 열기*로 한 번 허용하거나
  `xattr -dr com.apple.quarantine "Easy Typst Editor.app"`을 실행하세요.
- **Linux:** 웹뷰가 없으면 설치하세요 — Debian/Ubuntu `sudo apt install libwebkit2gtk-4.1-0`, Fedora `sudo dnf install webkit2gtk4.1`,
  Arch `sudo pacman -S webkit2gtk-4.1`. `sh register.sh`로 앱 메뉴에 넣고 .typ 더블클릭으로 열 수 있습니다.
- **Windows:** `register.ps1`을 실행하면 .typ 파일을 더블클릭으로 엽니다.

</details>

<br>

## ❓ 자주 묻는 질문

<details><summary><b>정말 무료인가요?</b></summary>

네. MIT 라이선스 오픈 소스입니다. 개인·회사·학교 어디서나 무료로 쓰고 고쳐서 배포해도 됩니다.
</details>

<details><summary><b>Typst를 전혀 몰라도 쓸 수 있나요?</b></summary>

네. 글을 고치고, 굵게 하고, 표에 줄을 넣는 일은 워드처럼 하면 됩니다. 수식만은 Typst 수식 문법으로 쓰지만, 기호 단추와 LaTeX 붙여넣기가 도와줍니다.
</details>

<details><summary><b>제 원고가 망가지지 않을까요?</b></summary>

고칠 때마다 구문을 검사하고, 조판이 깨지면 자동으로 되돌립니다. 저장하기 전에는 디스크에 쓰지 않고, 저장할 때마다 원본을 백업합니다.
Ctrl+Z로 몇 번이고 되돌릴 수 있습니다.
</details>

<details><summary><b>문서가 인터넷으로 나가나요?</b></summary>

기본으로는 아니요. Typst 조판 엔진(0.15.1)이 앱 안에 들어 있어 모든 일이 내 PC 안에서 일어납니다. 화면과 엔진은 내 PC 안(127.0.0.1)에서만 이야기하고,
그 통로도 실행할 때마다 새로 만드는 토큰으로 잠겨 있어 다른 웹사이트나 프로그램이 끼어들 수 없습니다.
편집기 자체는 인터넷에 접속하지 않습니다. 다만 「✨ AI」를 쓰면 내 컴퓨터의 Claude Code가 원고 내용을 Anthropic으로 보냅니다. 아래 질문을 보세요.
</details>

<details><summary><b>「✨ AI」로 고치기는 안전한가요? 무엇이 밖으로 나가나요?</b></summary>

「✨ AI」는 **내 컴퓨터에 이미 설치해 로그인해 둔 [Claude Code](https://claude.com/claude-code)** 를 불러 원고를 고치게 하는 선택 기능입니다.
Claude Code가 없으면 이 기능은 꺼져 있고, 편집기는 인터넷에 아무것도 보내지 않습니다.

- **편집기에는 키를 넣지 않습니다.** API 키·비밀번호를 받지도, 저장하지도, 보내지도 않습니다. 로그인은 Claude Code가 제 방식대로 합니다.
- **쓰는 법:** 「✨ AI」(Ctrl+K)에 "이 쪽 (가), (나) 상자 안 양 끝을 두 칸씩 띄워 줘. 해설도 마찬가지로."처럼 적으면, Claude Code가 템플릿과 원고를 읽고 필요한 파일을 고칩니다.
  편집기는 지금 보는 쪽·선택한 글이 원고의 어디인지 알려 주고, 끝나면 다시 조판해 바뀐 곳을 목록으로 보여 줍니다.
- **나가는 것:** 지시문과, Claude Code가 일하면서 읽은 **문서 폴더 안 파일의 내용**이 Anthropic으로 갑니다. 어느 파일을 읽을지는 Claude가 정하므로 **문서 폴더 전체가 전송될 수 있다**고 생각하세요. 사용량은 내 Claude 계정에서 나갑니다.
- **Claude Code에 주는 권한:** 제한 모드로만 부릅니다. 문서 폴더 안에서 파일을 **읽고·찾고·고치는 것**만 할 수 있고, 명령 실행·웹 접근·새 파일 만들기·파일 지우기는 할 수 없습니다. 내 Claude Code 설정·훅·MCP 서버도 불러오지 않습니다.
- **원고 보호:** Claude Code는 **저장된 파일을 직접** 고칩니다. 편집기는 고치기 전 파일을 백업 폴더에 남기고, 조판이 깨지면 두 번까지 다시 고치게 한 뒤 그래도 안 되면 모두 되돌립니다.
  바뀐 곳은 목록으로 나오고 「되돌리기」 한 번(또는 Ctrl+Z)으로 되돌립니다. 그래도 **조판이 된다고 내용이 옳은 것은 아닙니다.** 결과는 꼭 직접 확인하세요.
- **꼭 지켜 주세요:** ① 중요한 원고는 먼저 백업하거나 git으로 관리하세요. ② 남에게 받은, 믿을 수 없는 문서에는 AI 기능을 쓰지 마세요(문서 속 문장이 AI를 속일 수 있습니다).
  ③ 문서 폴더에 비밀번호·개인 정보 파일을 함께 두지 마세요. ④ 외부로 내보내면 안 되는 원고(시험 문제, 계약상 비밀 등)에는 쓰지 마세요.

자세한 내용은 [설명서의 「AI로 고치기」](docs/manual.ko.md#ai로-고치기-claude-code)와 아래 [면책사항](#disclaimer)에 있습니다.
</details>

<details><summary><b>`typst compile`로 만든 PDF와 똑같이 보이나요?</b></summary>

네. 같은 Typst 0.15.1 엔진과 같은 글꼴을 쓰므로 판형·쪽 나눔이 같습니다. 「PDF로 저장」(Ctrl+P)도 같은 엔진으로 만듭니다.
</details>

<details><summary><b>어떤 글은 왜 고쳐지지 않나요?</b></summary>

쪽 번호·차례·참조 번호처럼 템플릿이 계산해서 만드는 글은 원고에 그 글자가 없어 잠깁니다. 누르면 이유가 나오고,
덩어리째 옮기거나 지우려면 Alt+클릭으로 고르면 됩니다. 잠기면 안 될 글이 잠긴다면 [이슈](https://github.com/qnai-ai/Easy_Typst_Editor/issues/new/choose)로 알려 주세요.
</details>

<details><summary><b>단축키를 한눈에 보고 싶어요</b></summary>

macOS에서는 Ctrl 대신 ⌘, Alt 대신 ⌥를 씁니다. 앱에서 F1을 누르면 전체 표가 나옵니다.

| 키 | 하는 일 |
|---|---|
| 누르기 · 끌기 · Shift+방향키 | 커서 · 선택 |
| Ctrl+B · Ctrl+I · Ctrl+U | 굵게 · 기울임 · 밑줄 |
| Ctrl+Z · Ctrl+Y · Ctrl+S · Ctrl+P | 되돌리기 · 다시 · 저장 · PDF로 저장 |
| Ctrl+F · Ctrl+H | 찾기 · 바꾸기 |
| Ctrl+K | AI로 고치기: 내 컴퓨터의 Claude Code에게 지시(Claude Code가 설치되어 있을 때) |
| 수식 누르기 · Ctrl+M | 수식 고치기 · 새 수식 넣기 |
| Ctrl+V · Ctrl+Shift+V | 붙여넣기(LaTeX → Typst) · 그대로 붙여넣기 |
| 표 안에서 오른쪽 클릭 | 줄·칸 넣기와 지우기 |
| Alt+클릭 | 덩어리째 고르기(옮기기·지우기) |

</details>

<br>

## 🐞 오류·건의는 24시간 안에 고칩니다

이상한 점이나 바라는 기능은 **[GitHub Issues](https://github.com/qnai-ai/Easy_Typst_Editor/issues/new/choose)** 로 알려 주세요. 24시간 안에 고치겠습니다.
문제가 보이는 작은 `.typ` 파일이나 화면 캡처를 함께 주시면 훨씬 빨라집니다.
도움이 되셨다면 ⭐ 하나가 다른 Typst 사용자들이 이 도구를 찾는 데 큰 힘이 됩니다.

## 📘 더 알아보기

- **[자세한 설명서](docs/manual.ko.md)** — 모든 기능, 단축키 전체, 설계 원칙, 구조, 빌드, 알려진 한계
- 번역 기여: 화면 글은 모두 [`ui/i18n.json`](ui/i18n.json) 한 파일에 있습니다(한국어 · English · 日本語 · 简体中文 · Español).

<a id="disclaimer"></a>

## ⚠️ 면책사항

**이 프로그램을 내려받거나 실행하면 아래 내용에 동의한 것으로 봅니다. 동의하지 않으면 쓰지 마세요.**

1. **보증이 없습니다.** 이 프로그램은 개인이 만들어 무료로 공개한 오픈 소스이며 **"있는 그대로"** 제공됩니다. 상품성, 특정 목적에의 적합성, 오류가 없음, 결과의 정확성, 권리 비침해를 포함해 명시적이든 묵시적이든 **어떤 보증도 하지 않습니다.**
2. **책임을 지지 않습니다.** 원고의 손상·유실, 잘못된 조판과 인쇄 사고, 재인쇄 비용, 납기 지연, 업무 중단, 매출·이익의 손실, 데이터 유출, AI 서비스 요금, 제3자의 청구를 포함해, 이 프로그램을 쓰거나 쓰지 못해서 생긴 **직접·간접·부수적·특별·결과적 손해 일체**에 대해 만든 사람과 기여자는 법이 허용하는 가장 넓은 범위에서 **책임을 지지 않습니다.**
3. **백업은 사용자의 몫입니다.** 프로그램이 남기는 백업과 되돌리기 기능은 편의를 위한 것이지 보장이 아닙니다. 원고는 스스로 백업하거나 git 같은 판 관리 도구로 지키세요.
4. **결과를 확인할 책임은 사용자에게 있습니다.** 인쇄소에 넘기거나 배포·출판하기 전에 최종 PDF를 처음부터 끝까지 직접 확인하세요. 이 프로그램이 보여 준 지면이나 "저장했습니다"라는 안내를 근거로 한 판단의 결과는 사용자의 책임입니다.
5. **AI 기능은 더욱 그렇습니다.**
   - 「✨ AI」는 사용자가 따로 설치하고 로그인한 제3자 프로그램(Anthropic의 Claude Code)을 사용자의 지시로 실행합니다. **Claude Code는 내 컴퓨터의 원고 파일을 직접 고칩니다.**
   - AI가 내놓은 결과는 **틀릴 수 있고, 원고를 망가뜨릴 수 있으며, 시키지 않은 곳을 바꿀 수 있습니다.** 편집기의 안전장치(제한 모드, 조판 확인, 바뀐 곳 목록, 되돌리기, 백업)는 위험을 줄일 뿐 없애지 못합니다.
   - AI 기능을 쓰면 **원고 내용이 Anthropic으로 전송됩니다.** 그 원고를 외부 서비스에 보내도 되는지(기밀, 개인 정보, 시험 문제, 남의 저작물, 소속 기관의 규정, 계약상 의무)를 판단하고 필요한 권한을 얻는 일은 **전적으로 사용자의 책임**입니다.
   - 보낸 내용의 처리, 요금과 사용 한도, 서비스의 변경·중단에는 Anthropic의 약관과 개인정보 처리방침이 적용됩니다. 만든 사람은 이를 통제하지 못하며 그에 대해 책임지지 않습니다.
   - 문서 속에 숨은 문장이 AI의 행동을 바꾸는 공격(프롬프트 주입)을 완전히 막을 수는 없습니다. **출처를 믿을 수 없는 문서에는 AI 기능을 쓰지 마세요.**
6. **보안에 대한 보증도 없습니다.** 보안을 위해 여러 조치를 해 두었지만 취약점이 없다고 보증하지 않습니다. 이 프로그램의 사용과 관련해 생긴 보안 사고에 대해 책임지지 않습니다.
7. **지원 의무가 없습니다.** "24시간 안에 고칩니다"는 만든 사람의 노력 목표일 뿐 약속이나 보증이 아닙니다. 수정·유지 보수·호환성 유지의 의무는 없으며 기능은 예고 없이 바뀌거나 사라질 수 있습니다.
8. **다른 회사와 관계가 없습니다.** 이 프로그램은 Typst GmbH, Anthropic, Adobe와 제휴·후원·보증 관계가 없습니다. Typst, Claude, Claude Code는 각 권리자의 상표입니다.

법적 효력이 있는 조건의 전문은 [LICENSE](LICENSE)(MIT)에 있으며, 위 내용과 다를 때는 LICENSE가 우선합니다.

## 라이선스

[MIT](LICENSE). 오픈 소스 [Typst](https://github.com/typst/typst) 컴파일러(Apache-2.0)와 [tex2typst](https://github.com/qwinsi/tex2typst)(Apache-2.0, 라이선스: [`ui/vendor/tex2typst.LICENSE.txt`](ui/vendor/tex2typst.LICENSE.txt))를 담고 있습니다.
Typst GmbH 및 Adobe와는 관계가 없습니다.
