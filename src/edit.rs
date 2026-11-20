//! 원고(typst 소스) 글자열을 고치는 연산들.
//!
//! 모든 연산은 파일 하나의 본문과 커서 위치(바이트)를 받아 새 본문과 새 커서를 돌려준다.
//! 고친 뒤 구문 구조가 뜻하지 않게 달라지면 실패로 처리해, 원고를 망가뜨리지 않는다.

use std::collections::BTreeMap;
use std::ops::Range;

use typst::syntax::{parse, LinkedNode, Side, SyntaxKind, SyntaxNode};
use unicode_segmentation::UnicodeSegmentation;

pub type R<T> = Result<T, String>;

/// 글자 흐름 안에서 투명하게 다루는 꾸밈 함수들.
const INLINE: &[&str] = &[
    "strong", "emph", "underline", "u", "text", "smallcaps", "sub", "super", "highlight", "strike",
    "overline", "sm", "gt",
];

#[derive(Clone, Copy, PartialEq)]
pub enum Style { Bold, Italic, Under, Strike }

#[derive(Clone)]
pub struct Work {
    pub text: String,
    pub caret: usize,
    pub anchor: usize,
    /// 지울 글자 없이 글 상자의 가장자리에 닿았다(커서를 지면에서 옆 글로 옮겨야 한다).
    pub edge: bool,
    /// 사용자에게 알릴 말(통째로 지운 요소 이름 등).
    pub note: Option<String>,
}

impl Work {
    pub fn new(text: String, caret: usize, anchor: usize) -> Work { Work { text, caret, anchor, edge: false, note: None } }
    pub fn sel(&self) -> Option<(usize, usize)> {
        if self.caret == self.anchor { None } else { Some((self.caret.min(self.anchor), self.caret.max(self.anchor))) }
    }
    fn map(pos: usize, r: &Range<usize>, n: usize, after: bool) -> usize {
        if r.is_empty() {
            if pos > r.start || (pos == r.start && after) { pos + n } else { pos }
        } else if pos >= r.end { pos + n - r.len() }
        else if pos > r.start { r.start + n }
        else { pos }
    }
    /// 구간을 바꾼다. `after`: 그 자리에 있던 표시(커서)가 끼워 넣은 글 뒤로 가는지.
    fn rep(&mut self, r: Range<usize>, with: &str, after: bool) {
        self.caret = Self::map(self.caret, &r, with.len(), after);
        self.anchor = Self::map(self.anchor, &r, with.len(), after);
        self.text.replace_range(r, with);
    }
    fn eol(&self) -> &'static str { if self.text.contains("\r\n") { "\r\n" } else { "\n" } }
}

// ───────────────────────── 구문 도우미

fn tree(text: &str) -> SyntaxNode { parse(text) }
fn bad(node: &SyntaxNode) -> bool { !node.errors_and_warnings().0.is_empty() }

const SIG_SKIP: &[SyntaxKind] = &[
    SyntaxKind::Text, SyntaxKind::Space, SyntaxKind::Parbreak, SyntaxKind::Linebreak, SyntaxKind::Escape,
    SyntaxKind::Shorthand, SyntaxKind::SmartQuote, SyntaxKind::Markup, SyntaxKind::Semicolon,
];

type Sig = BTreeMap<u8, i32>;

fn add_sig(node: &SyntaxNode, sig: &mut Sig, sign: i32) {
    if !SIG_SKIP.contains(&node.kind()) { *sig.entry(node.kind() as u8).or_default() += sign; }
    for c in node.children() { add_sig(c, sig, sign); }
}
fn sig_of(node: &SyntaxNode) -> Sig {
    let mut s = Sig::new(); add_sig(node, &mut s, 1); s.retain(|_, v| *v != 0); s
}

fn is_markup_text(n: &LinkedNode) -> bool {
    n.kind() == SyntaxKind::Text && n.parent_kind() == Some(SyntaxKind::Markup)
}

/// `p`가 놓일 수 있는 가장 안쪽 Markup 노드의 범위.
fn markup_at(node: &LinkedNode, p: usize) -> Option<Range<usize>> {
    let r = node.range();
    if p < r.start || p > r.end { return None; }
    if node.kind() == SyntaxKind::Markup {
        for c in node.children() {
            let cr = c.range();
            if cr.start < p && p < cr.end {
                if matches!(c.kind(), SyntaxKind::Text | SyntaxKind::Space | SyntaxKind::Parbreak) { return Some(r); }
                return markup_at(&c, p);
            }
        }
        return Some(r);
    }
    for c in node.children() {
        let cr = c.range();
        if cr.start <= p && p <= cr.end {
            if let Some(m) = markup_at(&c, p) { return Some(m); }
        }
    }
    None
}

/// 문자열 리터럴 안이면 따옴표 안쪽 범위.
fn str_at(root: &LinkedNode, p: usize) -> Option<Range<usize>> {
    for side in [Side::Before, Side::After] {
        if let Some(l) = root.leaf_at(p, side) {
            let r = l.range();
            if l.kind() == SyntaxKind::Str && r.start < p && p < r.end { return Some(r.start + 1..r.end - 1); }
        }
    }
    None
}

pub enum Ctx { Markup, Str(Range<usize>), No }

fn ctx(root: &LinkedNode, p: usize) -> Ctx {
    if let Some(r) = str_at(root, p) { return Ctx::Str(r); }
    if markup_at(root, p).is_some() { Ctx::Markup } else { Ctx::No }
}

fn find_node<'a>(node: &LinkedNode<'a>, r: &Range<usize>, kind: SyntaxKind) -> Option<LinkedNode<'a>> {
    if node.range() == *r && node.kind() == kind { return Some(node.clone()); }
    for c in node.children() {
        let cr = c.range();
        if cr.start <= r.start && r.end <= cr.end {
            if let Some(f) = find_node(&c, r, kind) { return Some(f); }
        }
    }
    None
}

#[derive(Clone, Debug, PartialEq)]
enum WK { Star, Under, Func(String, bool) } // Func(이름, 다른 인자가 있는지)

#[derive(Clone, Debug)]
struct Wrap {
    open: Range<usize>,
    close: Range<usize>,
    content: Range<usize>,
    kind: WK,
    /// 글줄에 낀 것이 아니라 제 문단을 혼자 차지하는 덩어리인지
    blocky: bool,
    /// `#text(.., weight: ..)[..]`의 굵기: (굵은지, 그 인자의 범위, 인자가 그것뿐인지)
    weight: Option<(bool, Range<usize>, bool)>,
    /// 이 꾸밈의 노드 범위(함수 꼴이면 `#` 뒤의 부름)
    node: Range<usize>,
}

impl Wrap {
    fn is_style(&self, st: Style) -> bool {
        match (&self.kind, st) {
            (WK::Star, Style::Bold) => true,
            (WK::Under, Style::Italic) => true,
            (WK::Func(n, false), Style::Bold) => n == "strong",
            (WK::Func(n, false), Style::Italic) => n == "emph",
            (WK::Func(n, false), Style::Under) => n == "underline" || n == "u",
            (WK::Func(n, false), Style::Strike) => n == "strike",
            _ => false,
        }
    }
    fn inline(&self) -> bool {
        match &self.kind { WK::Func(n, _) => INLINE.contains(&n.as_str()), _ => true }
    }
    fn is_hang(&self, text: &str) -> bool {
        matches!(&self.kind, WK::Func(n, true) if n == "par") && text[self.open.clone()].contains("hanging-indent")
    }
    /// `#align(center)[..]`처럼 문단 하나를 감싼 정렬이면 그 정렬 이름.
    fn align_of<'a>(&self, text: &'a str) -> Option<&'a str> {
        if !matches!(&self.kind, WK::Func(n, true) if n == "align") { return None; }
        let how = text[self.open.clone()].strip_prefix("#align(")?.strip_suffix(")[")?;
        ["left", "center", "right"].contains(&how).then_some(how)
    }
    /// 문단을 감싼 꾸밈(내어쓰기·정렬): 글자 흐름에서는 없는 것처럼 다룬다.
    fn is_parwrap(&self, text: &str) -> bool { self.is_hang(text) || self.align_of(text).is_some() }
    /// 글줄 안에서 글자와 함께 흐르는 꾸밈인지: 커서와 지우기가 표식을 그냥 넘어간다.
    /// `#lk("a")[낱말]`처럼 템플릿이 만든 꾸밈도 글줄에 끼어 있으면 여기에 든다.
    fn flows(&self, text: &str) -> bool { self.inline() || self.is_parwrap(text) || !self.blocky }
}

/// 노드가 글을 감싸는 꾸밈(굵게·기울임·`#이름[..]`)이면 그 구조.
fn wrap_of(node: &LinkedNode) -> Option<Wrap> {
    match node.kind() {
        SyntaxKind::Strong | SyntaxKind::Emph => {
            let ch: Vec<_> = node.children().collect();
            if ch.len() < 2 { return None; }
            let (f, l) = (&ch[0], &ch[ch.len() - 1]);
            let mk = if node.kind() == SyntaxKind::Strong { SyntaxKind::Star } else { SyntaxKind::Underscore };
            if f.kind() != mk || l.kind() != mk { return None; }
            Some(Wrap {
                open: f.range(), close: l.range(), content: f.range().end..l.range().start,
                kind: if mk == SyntaxKind::Star { WK::Star } else { WK::Under },
                blocky: false,
                weight: None,
                node: node.range(),
            })
        }
        SyntaxKind::FuncCall => {
            let ch: Vec<_> = node.children().collect();
            let callee = ch.first()?;
            if callee.kind() != SyntaxKind::Ident { return None; }
            let args = ch.iter().find(|c| c.kind() == SyntaxKind::Args)?;
            let ach: Vec<_> = args.children().collect();
            let blocks: Vec<_> = ach.iter().filter(|c| c.kind() == SyntaxKind::ContentBlock).collect();
            if blocks.len() != 1 || ach.last()?.kind() != SyntaxKind::ContentBlock { return None; }
            let cb = blocks[0].range();
            let hash = node.prev_sibling_with_trivia().filter(|h| h.kind() == SyntaxKind::Hash)?;
            let name = callee.get().full_text().to_string();
            Some(Wrap {
                open: hash.range().start..cb.start + 1,
                close: cb.end - 1..cb.end,
                content: cb.start + 1..cb.end - 1,
                blocky: block_like(node, &name),
                weight: if name == "text" { weight_arg(&ach) } else { None },
                node: node.range(),
                kind: WK::Func(name, ach.len() > 1),
            })
        }
        _ => None,
    }
}

/// `text(..)`의 인자들에서 `weight:`를 찾는다: (굵은지, 인자의 범위, 인자가 그것뿐인지).
/// 값이 변수나 식이면 알 수 없으므로 None.
fn weight_arg(args: &[LinkedNode]) -> Option<(bool, Range<usize>, bool)> {
    let real = |k: SyntaxKind| !matches!(k, SyntaxKind::LeftParen | SyntaxKind::RightParen | SyntaxKind::Comma | SyntaxKind::Space | SyntaxKind::ContentBlock);
    let n = args.iter().filter(|c| real(c.kind())).count();
    for c in args {
        if c.kind() != SyntaxKind::Named { continue; }
        let ch: Vec<_> = c.children().collect();
        if ch.first().is_none_or(|x| x.get().full_text() != "weight") { continue; }
        let v = ch.last()?;
        let t = v.get().full_text();
        let bold = match v.kind() {
            SyntaxKind::Str => match t.trim_matches('"') {
                "semibold" | "bold" | "extrabold" | "black" => true,
                "thin" | "extralight" | "light" | "regular" | "medium" => false,
                _ => return None,
            },
            SyntaxKind::Int => t.parse::<i64>().ok()? >= 600,
            _ => return None,
        };
        return Some((bold, c.range(), n == 1));
    }
    None
}

/// `p`를 감싼 꾸밈들(안쪽부터). `p`가 내용 범위 안(경계 포함)에 있는 것만.
fn wraps_at(root: &LinkedNode, p: usize) -> Vec<Wrap> {
    let mut out = vec![];
    fn go(node: &LinkedNode, p: usize, out: &mut Vec<Wrap>) {
        for c in node.children() {
            let r = c.range();
            if r.start <= p && p <= r.end {
                if let Some(w) = wrap_of(&c) {
                    if w.content.start <= p && p <= w.content.end { out.push(w); }
                }
                go(&c, p, out);
            }
        }
    }
    go(root, p, &mut out);
    out.reverse();
    out
}

/// 글 상자(문단 단위 Markup)의 범위: 글자 꾸밈은 건너 올라간다.
fn block_at(root: &LinkedNode, text: &str, p: usize) -> Option<Range<usize>> {
    let mut m = markup_at(root, p)?;
    for w in wraps_at(root, p) {
        if w.content != m { continue; }
        if w.flows(text) {
            m = markup_at(root, w.open.start).filter(|o| o.start <= w.open.start && w.close.end <= o.end)?;
        } else { break; }
    }
    Some(m)
}

fn wordy(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric() && !matches!(c as u32, 0x1100..=0x11FF | 0x3040..=0x30FF | 0x3130..=0x318F | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF))
}
fn prev_char(t: &str, p: usize) -> Option<char> { t[..p].chars().next_back() }
fn next_char(t: &str, p: usize) -> Option<char> { t[p..].chars().next() }
fn needs_guard(c: Option<char>) -> bool { matches!(c, Some('(' | '[' | '.')) }

/// `p` 바로 앞에서 `#` 식(함수 부름·이름)이 끝나는지.
fn expr_ends_at(root: &LinkedNode, p: usize) -> Option<SyntaxKind> {
    let leaf = root.leaf_at(p, Side::Before)?;
    if leaf.range().end != p { return None; }
    let mut n = leaf;
    loop {
        if n.parent_kind() == Some(SyntaxKind::Markup) {
            if n.range().end == p && n.prev_sibling_with_trivia().is_some_and(|h| h.kind() == SyntaxKind::Hash) {
                return Some(n.kind());
            }
            return None;
        }
        n = n.parent()?.clone();
    }
}

// ───────────────────────── 글자 넣기

fn escape_markup(text: &str, p: usize, s: &str) -> String {
    let before = &text[..p];
    let line = before.rsplit('\n').next().unwrap_or("");
    let mut line_start = line.trim().is_empty() || before.trim_end().ends_with('[');
    let mut digits = !line.trim().is_empty() && line.trim().chars().all(|c| c.is_ascii_digit());
    let mut prev = prev_char(text, p);
    let after = next_char(text, p);
    let cs: Vec<char> = s.chars().collect();
    let mut out = String::new();
    for (i, &c) in cs.iter().enumerate() {
        let next = cs.get(i + 1).copied().or(after);
        let esc = match c {
            '\\' | '*' | '_' | '#' | '$' | '`' | '<' | '@' | '[' | ']' | '~' => true,
            '/' => prev == Some('/') || matches!(next, Some('/' | '*')) || line_start,
            '-' => prev == Some('-') || next == Some('-') || line_start,
            '=' | '+' => line_start,
            '.' => digits,
            _ => false,
        };
        let c = if c == '\t' || c == '\n' || c == '\r' { ' ' } else { c };
        if esc { out.push('\\'); }
        out.push(c);
        if !c.is_whitespace() {
            digits = (line_start || digits) && c.is_ascii_digit();
            line_start = false;
        }
        prev = Some(c);
    }
    out
}

/// 구조가 달라지지 않는 글자 고침. 굵게 표시(`*`) 바로 옆이라 구조가 깨지면 함수 꼴로 바꿔 다시 해 본다.
fn checked(w: &mut Work, r: Range<usize>, with: &str) -> R<()> {
    let old = tree(&w.text);
    let old_sig = sig_of(&old);
    let mut t = w.text.clone();
    t.replace_range(r.clone(), with);
    let new = tree(&t);
    if !bad(&new) && sig_of(&new) == old_sig {
        w.rep(r, with, true);
        return Ok(());
    }
    // 가까운 *굵게*, _기울임_ 을 #strong[..], #emph[..] 꼴로
    let root = LinkedNode::new(&old);
    let mut near: Vec<Wrap> = vec![];
    fn go(n: &LinkedNode, r: &Range<usize>, out: &mut Vec<Wrap>) {
        for c in n.children() {
            let cr = c.range();
            if cr.end + 2 < r.start || cr.start > r.end + 2 { continue; }
            if let Some(wp) = wrap_of(&c) {
                if wp.kind == WK::Star || wp.kind == WK::Under {
                    let close = |m: usize| m + 2 >= r.start && m <= r.end + 2;
                    if close(wp.open.start) || close(wp.close.start) { out.push(wp); }
                }
            }
            go(&c, r, out);
        }
    }
    go(&root, &r, &mut near);
    if near.is_empty() { return Err("이 자리는 지면에서 바로 고칠 수 없습니다.".into()); }
    let mut w2 = Work::new(w.text.clone(), r.start, r.end);
    let (c0, a0) = (w.caret, w.anchor);
    let mut extra = Work::new(String::new(), c0, a0);
    near.sort_by_key(|x| std::cmp::Reverse(x.open.start));
    for wp in near {
        let name = if wp.kind == WK::Star { "#strong[" } else { "#emph[" };
        let guard = if needs_guard(next_char(&w2.text, wp.close.end)) { "];" } else { "]" };
        for (rr, s) in [(wp.close.clone(), guard), (wp.open.clone(), name)] {
            let after = rr.start < r.start || (rr.start == r.start && rr.end <= r.start);
            extra.caret = Work::map(extra.caret, &rr, s.len(), after);
            extra.anchor = Work::map(extra.anchor, &rr, s.len(), after);
            // 고칠 구간(w2.caret..w2.anchor)은 표식 바깥에 그대로 있어야 한다
            w2.caret = Work::map(w2.caret, &rr, s.len(), rr.end <= r.start);
            w2.anchor = Work::map(w2.anchor, &rr, s.len(), rr.end <= r.start);
            w2.text.replace_range(rr, s);
        }
    }
    let conv = tree(&w2.text);
    if bad(&conv) { return Err("이 자리는 지면에서 바로 고칠 수 없습니다.".into()); }
    let conv_sig = sig_of(&conv);
    let r2 = w2.caret..w2.anchor;
    let mut t2 = w2.text.clone();
    t2.replace_range(r2.clone(), with);
    let new2 = tree(&t2);
    if bad(&new2) || sig_of(&new2) != conv_sig {
        return Err("이 자리는 지면에서 바로 고칠 수 없습니다.".into());
    }
    w.text = w2.text;
    w.caret = extra.caret;
    w.anchor = extra.anchor;
    w.rep(r2, with, true);
    Ok(())
}

fn insert_plain(w: &mut Work, s: &str) -> R<()> {
    if s.is_empty() { return Ok(()); }
    let p = w.caret;
    let node = tree(&w.text);
    let root = LinkedNode::new(&node);
    match ctx(&root, p) {
        Ctx::Str(_) => {
            let mut out = String::new();
            for c in s.chars() {
                match c { '\\' => out.push_str("\\\\"), '"' => out.push_str("\\\""), '\n' | '\r' | '\t' => out.push(' '), c => out.push(c) }
            }
            if out.contains('\\') { return Err("제목처럼 따옴표 안에 든 글에는 \\ 와 \" 를 넣을 수 없습니다.".into()); }
            checked(w, p..p, &out)
        }
        Ctx::Markup => {
            let mut esc = escape_markup(&w.text, p, s);
            if let Some(k) = expr_ends_at(&root, p) {
                let c = esc.chars().next();
                let ident_like = matches!(k, SyntaxKind::Ident | SyntaxKind::FieldAccess)
                    && c.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-');
                if needs_guard(c) || ident_like { esc.insert(0, ';'); }
            }
            checked(w, p..p, &esc)
        }
        Ctx::No => Err("이 자리에는 글자를 넣을 수 없습니다.".into()),
    }
}

/// 글자를 넣는다. 붙여넣기의 줄바꿈: 한 줄 바꿈은 강제 줄바꿈, 빈 줄은 새 문단.
pub fn insert(w: &mut Work, s: &str) -> R<()> {
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    // 선택한 글 위에 입력하면 그 글의 꾸밈(굵게 등)을 새 글이 물려받는다
    if w.sel().is_some() {
        delete_sel_opt(w, false)?;
        if s.is_empty() { cleanup_empty(w); return Ok(()); }
    }
    if !s.contains('\n') { return insert_plain(w, &s); }
    let mut blank = 0;
    let mut first = true;
    for line in s.split('\n') {
        if line.trim().is_empty() { if !first { blank += 1; } continue; }
        if !first {
            if blank >= 1 { enter(w)?; } else { linebreak(w)?; }
        }
        blank = 0; first = false;
        insert_plain(w, line)?;
    }
    Ok(())
}

// ───────────────────────── 지우기

enum Tg {
    /// 구간을 지운다(둘째 값: 구조가 그대로인지 확인할지).
    Del(Range<usize>, bool),
    /// 꾸밈 표식을 건너 커서만 옮긴다(지면에서는 같은 자리).
    Move(usize),
    /// 요소를 통째로 지운다(이름은 알림에 쓴다).
    Kill(Range<usize>, String),
    /// 글 상자의 가장자리: 여기서는 지울 글자가 없다. 커서를 지면에서 옆 글로 옮긴다.
    Edge,
}

fn func_of<'a>(leaf: &LinkedNode<'a>) -> Option<LinkedNode<'a>> {
    let mut n = leaf.parent()?.clone();
    for _ in 0..3 {
        if n.kind() == SyntaxKind::FuncCall { return Some(n); }
        n = n.parent()?.clone();
    }
    None
}
fn callee_name(f: &LinkedNode) -> String {
    f.children().next().map(|c| c.get().full_text().to_string()).unwrap_or_default()
}
fn hash_start(f: &LinkedNode) -> Option<usize> {
    f.prev_sibling_with_trivia().filter(|h| h.kind() == SyntaxKind::Hash).map(|h| h.range().start)
}

fn blank(k: SyntaxKind) -> bool {
    matches!(k, SyntaxKind::Space | SyntaxKind::Parbreak | SyntaxKind::LineComment | SyntaxKind::BlockComment)
}

/// 요소 안에 지면에서 고칠 본문 글(대괄호 안의 글)이 있는지.
fn has_body(f: &LinkedNode) -> bool {
    for c in f.children() {
        if c.kind() == SyntaxKind::Markup && c.children().any(|m| !blank(m.kind())) { return true; }
        if has_body(&c) { return true; }
    }
    false
}

/// 글 상자는 있는데 속이 모두 빈 요소인지(`#ex[][]`, `#nt("문맥")[]` 따위).
fn hollow(f: &LinkedNode) -> bool {
    fn go(n: &LinkedNode, blocks: &mut usize) -> bool {
        for c in n.children() {
            if c.kind() != SyntaxKind::Markup {
                if !go(&c, blocks) { return false; }
                continue;
            }
            *blocks += 1;
            for m in c.children() {
                match m.kind() {
                    k if blank(k) => {}
                    SyntaxKind::Hash | SyntaxKind::Semicolon => {}
                    SyntaxKind::FuncCall => {
                        let mut b = 0;
                        if !go(&m, &mut b) || b == 0 { return false; }
                    }
                    _ => return false,
                }
            }
        }
        true
    }
    let mut blocks = 0;
    go(f, &mut blocks) && blocks > 0
}

/// 대괄호(`[`·`]`)가 속한 글 상자가 비었는지.
fn empty_block(bracket: &LinkedNode) -> bool {
    bracket.parent().is_some_and(|cb| {
        cb.kind() == SyntaxKind::ContentBlock
            && cb.children().filter(|c| c.kind() == SyntaxKind::Markup).all(|m| m.children().all(|x| blank(x.kind())))
    })
}

/// `#요소`를 지울 범위: `#`부터 뒤의 `;`까지, 제 줄을 혼자 차지하면 줄째로.
fn kill_range(text: &str, f: &LinkedNode) -> Option<Range<usize>> {
    let h = hash_start(f)?;
    let mut end = f.range().end;
    if text[end..].starts_with(';') { end += 1; }
    let ls = text[..h].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let le = text[end..].find('\n').map(|i| end + i + 1).unwrap_or(text.len());
    if text[ls..h].trim().is_empty() && text[end..le].trim().is_empty() { return Some(ls..le); }
    Some(h..end)
}

/// `stack(check[가], check[나])`의 `check[..]`처럼, `#` 없이 쉼표로 늘어놓은 항목으로 쓰인 함수 부름인지.
fn is_item(f: &LinkedNode) -> bool {
    f.kind() == SyntaxKind::FuncCall && hash_start(f).is_none()
        && matches!(f.parent_kind(), Some(SyntaxKind::Args | SyntaxKind::Array))
}

/// 이름이 `name`인 함수가 이 파일에서 `..항목` 꼴로 인자를 몇 개든 받게 정의되었는지.
fn has_sink(root: &LinkedNode, name: &str) -> bool {
    if root.kind() == SyntaxKind::Closure {
        let ch: Vec<_> = root.children().collect();
        if ch.first().is_some_and(|c| c.kind() == SyntaxKind::Ident && c.get().full_text() == name) {
            return ch.iter().any(|c| c.kind() == SyntaxKind::Params && c.children().any(|p| p.kind() == SyntaxKind::Spread));
        }
    }
    root.children().any(|c| has_sink(&c, name))
}

/// 항목을 통째로 빼도 되는 자리인지: 항목을 몇 개든 받는 곳이어야 한다
/// (`stack`·`list`·`enum`, 배열, `..항목`을 받는 템플릿 함수). 표·격자는 칸이 밀리므로 빼지 않는다.
fn item_removable(root: &LinkedNode, f: &LinkedNode) -> bool {
    if !is_item(f) { return false; }
    let Some(par) = f.parent() else { return false };
    if par.kind() == SyntaxKind::Array { return true; }
    let Some(owner) = par.parent().filter(|o| o.kind() == SyntaxKind::FuncCall) else { return false };
    let name = callee_name(owner);
    matches!(name.as_str(), "stack" | "list" | "enum") || has_sink(root, &name)
}

/// 항목을 지울 범위: 뒤의 쉼표까지(마지막 항목이면 앞의 쉼표부터), 제 줄을 혼자 차지하면 줄째로.
fn item_range(text: &str, f: &LinkedNode) -> Range<usize> {
    let r = f.range();
    let (mut s, mut e) = (r.start, r.end);
    let rest = text[e..].trim_start_matches([' ', '\t']);
    if rest.starts_with(',') {
        e = text.len() - rest.len() + 1;
        let ls = text[..s].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let le = text[e..].find('\n').map(|i| e + i + 1).unwrap_or(text.len());
        if text[ls..s].trim().is_empty() && text[e..le].trim().is_empty() { return ls..le; }
        if text[e..].starts_with(' ') { e += 1; }
    } else {
        let head = text[..s].trim_end();
        if head.ends_with(',') { s = head.len() - 1; }
    }
    s..e
}

/// 인자가 글 상자뿐인 부름인지(`check[]`, `ex[][]`): 그림 경로 같은 다른 인자가 있으면 속이 비었다고 지우지 않는다.
fn bare_call(f: &LinkedNode) -> bool {
    f.children().filter(|c| c.kind() == SyntaxKind::Args)
        .all(|a| a.children().all(|c| c.kind() == SyntaxKind::ContentBlock))
}

/// 항목의 맨 앞(뒤)에서 지울 때: 바로 앞(뒤)에 속이 빈 항목이 있으면 그것을 걷어 낸다.
/// 빈 항목에는 글자가 없어 커서를 놓을 수 없으므로, 옆 항목에서 지워 들어가게 한다.
fn empty_sibling(root: &LinkedNode, text: &str, bracket: &LinkedNode, fwd: bool) -> Option<Tg> {
    let f = func_of(bracket)?;
    if !is_item(&f) { return None; }
    // 이 부름의 첫(마지막) 글 상자의 가장자리여야 한다
    let cb = bracket.parent()?;
    let blocks: Vec<_> = cb.parent()?.children().filter(|c| c.kind() == SyntaxKind::ContentBlock).collect();
    let edge = if fwd { blocks.last() } else { blocks.first() };
    if edge.map(|b| b.range()) != Some(cb.range()) { return None; }
    fn step<'a>(n: &LinkedNode<'a>, fwd: bool) -> Option<LinkedNode<'a>> { if fwd { n.next_sibling() } else { n.prev_sibling() } }
    let mut sib = step(&f, fwd)?;
    if sib.kind() == SyntaxKind::Comma { sib = step(&sib, fwd)?; }
    if is_item(&sib) && bare_call(&sib) && hollow(&sib) && item_removable(root, &sib) {
        return Some(Tg::Kill(item_range(text, &sib), callee_name(&sib)));
    }
    None
}

/// 공백·문단 나눔의 앞이나 뒤에 따로 선 덩어리(블록 요소, 설정문 등)가 붙어 있는지.
/// 그런 자리의 공백은 지우지 않는다(덩어리가 앞뒤 문단에 달라붙지 않게).
fn beside_solid(l: &LinkedNode, text: &str) -> bool {
    // 같은 줄의 띄어쓰기 한 칸은 언제나 지울 수 있다(덩어리는 줄을 달리해 선다)
    if l.kind() == SyntaxKind::Space && !l.get().full_text().contains('\n') { return false; }
    let solid = |n: &LinkedNode| -> bool {
        if !n.prev_sibling_with_trivia().is_some_and(|h| h.kind() == SyntaxKind::Hash) { return false; }
        match n.kind() {
            SyntaxKind::FuncCall => match wrap_of(n) {
                Some(wp) => !wp.flows(text),
                None => block_like(n, &callee_name(n)),
            },
            SyntaxKind::Ident | SyntaxKind::FieldAccess => false,
            _ => true,
        }
    };
    let prev = match l.prev_sibling() { Some(s) if s.kind() == SyntaxKind::Semicolon => s.prev_sibling(), o => o };
    if prev.is_some_and(|n| solid(&n)) { return true; }
    match l.next_sibling() {
        Some(h) if h.kind() == SyntaxKind::Hash => h.next_sibling_with_trivia().is_some_and(|n| solid(&n)),
        _ => false,
    }
}

/// 잎에서 올라가 마크업의 직계 자식이 되는 노드.
fn markup_child<'a>(leaf: &LinkedNode<'a>) -> Option<LinkedNode<'a>> {
    let mut n = leaf.clone();
    for _ in 0..12 {
        if n.parent_kind() == Some(SyntaxKind::Markup) { return Some(n); }
        n = n.parent()?.clone();
    }
    None
}

/// 글이 다 지워진 따옴표 문자열만 남은 요소(`#sub("")`)면 통째로 지운다.
/// 대괄호 글이 있거나 다른 문자열 인자에 글이 남아 있으면 그대로 둔다.
fn dead_str(root: &LinkedNode, text: &str, p: usize) -> Option<Tg> {
    let leaf = root.leaf_at(p, Side::Before)?;
    if leaf.kind() != SyntaxKind::Str || leaf.range().len() != 2 { return None; }
    let f = func_of(&leaf)?;
    fn live(n: &LinkedNode) -> bool {
        n.children().any(|c| (c.kind() == SyntaxKind::Str && c.range().len() > 2) || live(&c))
    }
    if has_body(&f) || live(&f) { return None; }
    kill_range(text, &f).map(|k| Tg::Kill(k, callee_name(&f)))
}

fn back_target(root: &LinkedNode, text: &str, p: usize) -> Tg {
    if let Some(sr) = str_at(root, p) {
        if p <= sr.start { return dead_str(root, text, p).unwrap_or(Tg::Edge); }
        let g = text[sr.start..p].graphemes(true).next_back().map(|g| g.len()).unwrap_or(0);
        return Tg::Del(p - g..p, true);
    }
    let Some(leaf) = root.leaf_at(p, Side::Before) else { return Tg::Edge };
    let r = leaf.range();
    let in_markup = leaf.parent_kind() == Some(SyntaxKind::Markup);
    match leaf.kind() {
        SyntaxKind::Text if in_markup => {
            let g = text[r.start..p].graphemes(true).next_back().map(|g| g.len()).unwrap_or(0);
            Tg::Del(p - g..p, true)
        }
        SyntaxKind::Space | SyntaxKind::Parbreak if in_markup => {
            if leaf.index() == 0 || beside_solid(&leaf, text) { Tg::Edge } else { Tg::Del(r.start..p, true) }
        }
        SyntaxKind::Escape | SyntaxKind::Shorthand | SyntaxKind::SmartQuote if in_markup => Tg::Del(r, true),
        SyntaxKind::Linebreak if in_markup => {
            let end = if text[p..].starts_with(' ') { p + 1 } else { p };
            Tg::Del(r.start..end, true)
        }
        SyntaxKind::Semicolon => Tg::Move(r.start),
        SyntaxKind::Star | SyntaxKind::Underscore => match leaf.parent().and_then(wrap_of) {
            Some(_) => Tg::Move(r.start),
            None => Tg::Edge,
        },
        // `]` 바로 뒤: 글줄에 낀 꾸밈이면 그 안으로, 글이 없는 요소면 통째로
        SyntaxKind::RightBracket => match func_of(&leaf) {
            Some(f) => {
                if let Some(wp) = wrap_of(&f) {
                    if wp.close == r && wp.flows(text) { return Tg::Move(r.start); }
                }
                if f.range().end == p && !has_body(&f) {
                    if let Some(k) = kill_range(text, &f) { return Tg::Kill(k, callee_name(&f)); }
                }
                Tg::Edge
            }
            None => Tg::Edge,
        },
        // 글 상자의 맨 앞
        SyntaxKind::LeftBracket => match func_of(&leaf) {
            Some(f) => {
                // 빈 글 상자에서 한 번 더 지우면, 속이 다 빈 요소는 통째로 걷어 낸다
                if empty_block(&leaf) && hollow(&f) {
                    if let Some(k) = kill_range(text, &f) { return Tg::Kill(k, callee_name(&f)); }
                    // `stack(check[], ..)`처럼 항목으로 쓰인 부름
                    if item_removable(root, &f) { return Tg::Kill(item_range(text, &f), callee_name(&f)); }
                }
                if let Some(wp) = wrap_of(&f) {
                    if wp.content.start == r.end && wp.flows(text) { return Tg::Move(wp.open.start); }
                }
                if let Some(t) = empty_sibling(root, text, &leaf, false) { return t; }
                Tg::Edge
            }
            None => Tg::Edge,
        },
        SyntaxKind::RightParen => match func_of(&leaf) {
            Some(f) if f.range().end == p && hash_start(&f).is_some() => {
                if callee_name(&f) == "h" {
                    let end = if text[p..].starts_with(';') { p + 1 } else { p };
                    Tg::Del(hash_start(&f).unwrap()..end, false)
                } else if !has_body(&f) {
                    match kill_range(text, &f) { Some(k) => Tg::Kill(k, callee_name(&f)), None => Tg::Edge }
                } else { Tg::Edge }
            }
            _ => Tg::Edge,
        },
        // `#이름`처럼 글줄에 낀 낱개 요소는 통째로
        _ => match markup_child(&leaf) {
            Some(n) if matches!(n.kind(), SyntaxKind::Ident | SyntaxKind::FieldAccess) && n.range().end == p => match hash_start(&n) {
                Some(h) => {
                    let end = if text[p..].starts_with(';') { p + 1 } else { p };
                    Tg::Kill(h..end, text[n.range()].to_string())
                }
                None => Tg::Edge,
            },
            _ => Tg::Edge,
        },
    }
}

fn fwd_target(root: &LinkedNode, text: &str, p: usize) -> Tg {
    if let Some(sr) = str_at(root, p) {
        if p >= sr.end { return dead_str(root, text, p).unwrap_or(Tg::Edge); }
        let g = text[p..sr.end].graphemes(true).next().map(|g| g.len()).unwrap_or(0);
        return Tg::Del(p..p + g, true);
    }
    let Some(leaf) = root.leaf_at(p, Side::After) else { return Tg::Edge };
    let r = leaf.range();
    let in_markup = leaf.parent_kind() == Some(SyntaxKind::Markup);
    let last = |l: &LinkedNode| l.parent().map(|p| p.children().len() == l.index() + 1).unwrap_or(false);
    match leaf.kind() {
        SyntaxKind::Text if in_markup => {
            let g = text[p..r.end].graphemes(true).next().map(|g| g.len()).unwrap_or(0);
            Tg::Del(p..p + g, true)
        }
        SyntaxKind::Space | SyntaxKind::Parbreak if in_markup => {
            if last(&leaf) || beside_solid(&leaf, text) { Tg::Edge } else { Tg::Del(p..r.end, true) }
        }
        SyntaxKind::Escape | SyntaxKind::Shorthand | SyntaxKind::SmartQuote if in_markup => Tg::Del(r, true),
        SyntaxKind::Linebreak if in_markup => {
            let end = if text[r.end..].starts_with(' ') { r.end + 1 } else { r.end };
            Tg::Del(r.start..end, true)
        }
        SyntaxKind::Semicolon => Tg::Move(r.end),
        SyntaxKind::Star | SyntaxKind::Underscore => match leaf.parent().and_then(wrap_of) {
            Some(_) => Tg::Move(r.end),
            None => Tg::Edge,
        },
        SyntaxKind::Hash => match leaf.next_sibling_with_trivia() {
            Some(f) if f.kind() == SyntaxKind::FuncCall => {
                let mut end = f.range().end;
                if text[end..].starts_with(';') { end += 1; }
                match wrap_of(&f) {
                    Some(wp) if wp.flows(text) => Tg::Move(wp.open.end),
                    _ if callee_name(&f) == "h" => Tg::Del(r.start..end, false),
                    _ if !has_body(&f) => match kill_range(text, &f) { Some(k) => Tg::Kill(k, callee_name(&f)), None => Tg::Edge },
                    _ => Tg::Edge,
                }
            }
            Some(n) if matches!(n.kind(), SyntaxKind::Ident | SyntaxKind::FieldAccess) => {
                let mut end = n.range().end;
                if text[end..].starts_with(';') { end += 1; }
                Tg::Kill(r.start..end, text[n.range()].to_string())
            }
            _ => Tg::Edge,
        },
        // 글 상자의 맨 끝
        SyntaxKind::RightBracket => match func_of(&leaf) {
            Some(f) => {
                if empty_block(&leaf) && hollow(&f) {
                    if let Some(k) = kill_range(text, &f) { return Tg::Kill(k, callee_name(&f)); }
                    if item_removable(root, &f) { return Tg::Kill(item_range(text, &f), callee_name(&f)); }
                }
                if let Some(wp) = wrap_of(&f) {
                    if wp.close == r && wp.flows(text) { return Tg::Move(r.end); }
                }
                if let Some(t) = empty_sibling(root, text, &leaf, true) { return t; }
                Tg::Edge
            }
            None => Tg::Edge,
        },
        _ => Tg::Edge,
    }
}

fn unchecked(w: &mut Work, r: Range<usize>, with: &str, after: bool) -> R<()> {
    let mut t = w.text.clone();
    t.replace_range(r.clone(), with);
    if bad(&tree(&t)) { return Err("이 자리는 지면에서 바로 고칠 수 없습니다.".into()); }
    w.rep(r, with, after);
    Ok(())
}

/// 구간을 지운다. 지운 자리에서 앞뒤가 달라붙어 뜻이 바뀌면 고쳐서 지운다:
/// `#이름` 뒤에 글자가 붙으면 `;`를 두고, 줄 머리 기호(`-`, `=` 등)가 드러나면 `\`를 붙인다.
fn del_checked(w: &mut Work, r: Range<usize>) -> R<()> {
    let glue = {
        let node = tree(&w.text);
        let root = LinkedNode::new(&node);
        match expr_ends_at(&root, r.start) {
            Some(k) => {
                let c = next_char(&w.text, r.end);
                let ident_like = matches!(k, SyntaxKind::Ident | SyntaxKind::FieldAccess)
                    && c.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-');
                Some(needs_guard(c) || ident_like)
            }
            None => None,
        }
    };
    if glue == Some(true) { return checked(w, r, ";"); }
    let first = match checked(w, r.clone(), "") { Ok(()) => return Ok(()), Err(e) => e };
    if glue.is_some() && checked(w, r.clone(), ";").is_ok() { return Ok(()); }
    if next_char(&w.text, r.end).is_some_and(|c| "-+=/.*_#$<@[]~`:".contains(c)) && checked(w, r, "\\").is_ok() {
        // 커서는 `\` 앞에 둔다(`\`와 그 글자는 한 덩어리)
        w.caret -= 1;
        w.anchor = w.caret;
        return Ok(());
    }
    Err(first)
}

/// 내용이 비어 버린 꾸밈(`**`, `#u[]`, 글줄에 낀 `#lk("a")[]`)을 걷어 낸다.
fn cleanup_empty(w: &mut Work) {
    for _ in 0..6 {
        let node = tree(&w.text);
        let root = LinkedNode::new(&node);
        let p = w.caret;
        let Some(wp) = wraps_at(&root, p).into_iter().next() else { return };
        if !wp.content.is_empty() || !wp.flows(&w.text) { return; }
        let mut end = wp.close.end;
        if matches!(wp.kind, WK::Func(..)) && w.text[end..].starts_with(';') { end += 1; }
        let mut t = w.text.clone();
        t.replace_range(wp.open.start..end, "");
        if bad(&tree(&t)) { return; }
        if let WK::Func(n, _) = &wp.kind {
            if !wp.inline() && !wp.is_parwrap(&w.text) { w.note = Some(n.clone()); }
        }
        w.rep(wp.open.start..end, "", false);
        w.anchor = w.caret;
    }
}

fn del_one(w: &mut Work, fwd: bool) -> R<()> {
    let mut p = w.caret;
    w.edge = false;
    for _ in 0..12 {
        let node = tree(&w.text);
        let root = LinkedNode::new(&node);
        let tg = if fwd { fwd_target(&root, &w.text, p) } else { back_target(&root, &w.text, p) };
        match tg {
            Tg::Move(q) => { if q == p { break; } p = q; }
            Tg::Edge => break,
            Tg::Kill(r, name) => {
                // 통째로 지울 수 없는 자리면 건너뛴다
                if unchecked(w, r.clone(), "", false).is_err() { break; }
                let q = if p >= r.end { p - r.len() } else { p.min(r.start) };
                w.caret = q; w.anchor = q;
                w.note = Some(name);
                return Ok(());
            }
            Tg::Del(r, check) => {
                if r.is_empty() { break; }
                w.caret = p; w.anchor = p;
                if check { del_checked(w, r)?; } else { unchecked(w, r, "", false)?; }
                w.anchor = w.caret;
                cleanup_empty(w);
                return Ok(());
            }
        }
    }
    w.edge = true;
    Ok(())
}

pub fn backspace(w: &mut Work) -> R<()> { if w.sel().is_some() { delete_sel(w) } else { del_one(w, false) } }
pub fn delete(w: &mut Work) -> R<()> { if w.sel().is_some() { delete_sel(w) } else { del_one(w, true) } }

/// Ctrl+Backspace / Ctrl+Delete: 낱말 하나(와 그 옆 공백)를 지운다.
pub fn del_word(w: &mut Work, fwd: bool) -> R<()> {
    if w.sel().is_some() { return delete_sel(w); }
    let p = w.caret;
    let q = {
        let node = tree(&w.text);
        let root = LinkedNode::new(&node);
        let text = &w.text;
        // 한 줄 안의 낱말 사이 공백(지워도 되는 것)
        let space = |q: usize, side: Side| -> Option<Range<usize>> {
            let back = matches!(side, Side::Before);
            let l = root.leaf_at(q, side)?;
            // 글 상자의 맨 앞(뒤로 지울 때)·맨 뒤(앞으로 지울 때) 공백은 건드리지 않는다
            let inner = if back { l.index() > 0 } else { l.parent().is_some_and(|m| m.children().len() > l.index() + 1) };
            let ok = l.kind() == SyntaxKind::Space && l.parent_kind() == Some(SyntaxKind::Markup)
                && !text[l.range()].contains('\n') && inner && !beside_solid(&l, text);
            ok.then(|| l.range())
        };
        // 낱말이 든 글 조각(따옴표 문자열 안쪽 또는 본문 글자 잎)
        let piece = |q: usize, side: Side| -> Option<Range<usize>> {
            if let Some(sr) = str_at(&root, q) { return Some(sr); }
            let l = root.leaf_at(q, side)?;
            is_markup_text(&l).then(|| l.range())
        };
        let mut q = p;
        if fwd {
            if let Some(r) = piece(q, Side::After) {
                let t = &text[q..r.end];
                let lead = t.len() - t.trim_start().len();
                q += lead + t[lead..].split_word_bounds().next().map(|s| s.len()).unwrap_or(0);
                let rest = &text[q..r.end];
                q += rest.len() - rest.trim_start().len();
            }
            if let Some(r) = space(q, Side::After) { q = r.end; }
        } else {
            if let Some(r) = space(q, Side::Before) { q = r.start; }
            if let Some(r) = piece(q, Side::Before) {
                let t = text[r.start..q].trim_end();
                q = r.start + t.len() - t.split_word_bounds().next_back().map(|s| s.len()).unwrap_or(0);
            }
        }
        q
    };
    if q == p { return del_one(w, fwd); }
    let r = q.min(p)..q.max(p);
    w.edge = false;
    w.caret = r.start; w.anchor = r.start;
    // 한꺼번에는 못 지우는 자리면(꾸밈 표식 옆 등) 한 글자 지우기로 물러선다
    if del_checked(w, r).is_err() {
        w.caret = p; w.anchor = p;
        return del_one(w, fwd);
    }
    w.anchor = w.caret;
    cleanup_empty(w);
    Ok(())
}

const MIXED: &str = "제목처럼 따옴표 안에 든 글과 본문에 걸친 선택은 한꺼번에 고칠 수 없습니다. 따로따로 선택해 주세요.";

/// 선택의 두 끝이 함께 고칠 수 있는 자리인지: 따옴표 문자열은 같은 문자열 안에서만.
fn sel_ok(root: &LinkedNode, a: usize, b: usize) -> R<()> {
    match (str_at(root, a), str_at(root, b)) {
        (Some(x), Some(y)) if x == y => Ok(()),
        (None, None) => Ok(()),
        _ => Err(MIXED.into()),
    }
}

/// 선택에 통째로 들어가도 지우지 않는 것: 설정·정의·불러오기(지면에 보이지 않는 원고의 뼈대).
fn keep_whole(c: &LinkedNode) -> bool {
    let frame = |k: SyntaxKind| matches!(k, SyntaxKind::LetBinding | SyntaxKind::SetRule | SyntaxKind::ShowRule
        | SyntaxKind::ModuleImport | SyntaxKind::ModuleInclude | SyntaxKind::LineComment | SyntaxKind::BlockComment);
    if frame(c.kind()) { return true; }
    match c.kind() {
        SyntaxKind::Hash => c.next_sibling_with_trivia().is_some_and(|n| frame(n.kind())),
        SyntaxKind::Semicolon => c.prev_sibling_with_trivia().is_some_and(|n| frame(n.kind())),
        _ => false,
    }
}

/// 선택 구간에서 지울 조각을 모은다(꾸밈 표식과 글 상자의 틀은 남긴다).
fn collect_del(node: &LinkedNode, a: usize, b: usize, out: &mut Vec<Range<usize>>, sig: &mut Sig) {
    for c in node.children() {
        let r = c.range();
        if r.end <= a || r.start >= b { continue; }
        if a <= r.start && r.end <= b {
            if keep_whole(&c) { continue; }
            // 남기는 것(설정문·주석)의 앞뒤 공백도 남긴다: 줄이 달라붙으면 뜻이 바뀐다
            if matches!(c.kind(), SyntaxKind::Space | SyntaxKind::Parbreak)
                && (c.prev_sibling_with_trivia().is_some_and(|n| keep_whole(&n)) || c.next_sibling_with_trivia().is_some_and(|n| keep_whole(&n))) { continue; }
            add_sig(c.get(), sig, -1);
            out.push(r);
        } else if matches!(c.kind(), SyntaxKind::Text | SyntaxKind::Space | SyntaxKind::Parbreak) {
            out.push(a.max(r.start)..b.min(r.end));
        } else {
            descend_del(&c, a, b, out, sig);
        }
    }
}
fn descend_del(node: &LinkedNode, a: usize, b: usize, out: &mut Vec<Range<usize>>, sig: &mut Sig) {
    for c in node.children() {
        let r = c.range();
        if r.end <= a || r.start >= b { continue; }
        if c.kind() == SyntaxKind::Markup { collect_del(&c, a, b, out, sig); } else { descend_del(&c, a, b, out, sig); }
    }
}

/// 선택한 글을 지운다. 여러 글 상자에 걸쳐 있으면 상자마다 그 안의 글만 지우고 틀은 남긴다.
pub fn delete_sel(w: &mut Work) -> R<()> { delete_sel_opt(w, true) }

/// `tidy`가 거짓이면 비어 버린 꾸밈을 걷지 않는다(선택 위에 바로 입력할 때: 새 글이 그 꾸밈을 물려받는다).
fn delete_sel_opt(w: &mut Work, tidy: bool) -> R<()> {
    if w.sel().is_none() { return Ok(()); }
    w.edge = false;
    if delete_sel_once(w, tidy).is_ok() { return Ok(()); }
    // *굵게* 표식이 낱말에 붙어 깨지는 경우: 함수 꼴로 바꾼 뒤 다시
    let (a, b) = w.sel().unwrap();
    let node = tree(&w.text);
    let root = LinkedNode::new(&node);
    let mut ws = vec![];
    all_wraps(&root, &mut ws);
    ws.retain(|x| matches!(x.kind, WK::Star | WK::Under) && x.open.start <= b + 1 && x.close.end + 1 >= a);
    if ws.is_empty() { return delete_sel_once(w, tidy); }
    let saved = (w.text.clone(), w.caret, w.anchor);
    w.anchor = a; w.caret = b;
    ws.sort_by_key(|x| std::cmp::Reverse(x.open.start));
    for wp in &ws { to_func(w, wp); }
    let r = delete_sel_once(w, tidy);
    if r.is_err() { w.text = saved.0; w.caret = saved.1; w.anchor = saved.2; }
    r
}

fn delete_sel_once(w: &mut Work, tidy: bool) -> R<()> {
    let Some((a, b)) = w.sel() else { return Ok(()) };
    let node = tree(&w.text);
    let root = LinkedNode::new(&node);
    sel_ok(&root, a, b)?;
    if str_at(&root, a).is_some() {
        w.caret = a; w.anchor = a;
        return checked(w, a..b, "").map(|_| { w.caret = a; w.anchor = a; });
    }
    let mut out = vec![];
    let mut expect = sig_of(&node);
    collect_del(&root, a, b, &mut out, &mut expect);
    expect.retain(|_, v| *v != 0);
    out.sort_by_key(|r| std::cmp::Reverse(r.start));
    let mut t = w.text.clone();
    for r in &out { t.replace_range(r.clone(), ""); }
    let new = tree(&t);
    if bad(&new) || sig_of(&new) != expect {
        return Err("이 선택은 지면에서 한꺼번에 지울 수 없습니다. 더 좁게 선택해 주세요.".into());
    }
    w.text = t; w.caret = a; w.anchor = a;
    if tidy { cleanup_empty(w); }
    Ok(())
}

// ───────────────────────── 문단·줄

/// Enter: 새 문단. 굵게·밑줄 안이면 꾸밈을 닫았다가 다시 연다.
pub fn enter(w: &mut Work) -> R<()> {
    if w.sel().is_some() { delete_sel(w)?; }
    let node = tree(&w.text);
    let root = LinkedNode::new(&node);
    let mut p = w.caret;
    match ctx(&root, p) {
        Ctx::Markup => {}
        Ctx::Str(_) => return Err("제목처럼 따옴표 안에 든 글에서는 문단을 나눌 수 없습니다.".into()),
        Ctx::No => return Err("이 자리에서는 문단을 나눌 수 없습니다.".into()),
    }
    let splittable = |wp: &Wrap| wp.inline() || wp.is_parwrap(&w.text);
    // 꾸밈의 맨 앞·맨 뒤라면 그 바깥으로
    loop {
        let Some(wp) = wraps_at(&root, p).into_iter().next() else { break };
        if !splittable(&wp) { break; }
        if p == wp.content.start { p = wp.open.start; }
        else if p == wp.content.end {
            p = wp.close.end;
            if matches!(wp.kind, WK::Func(..)) && w.text[p..].starts_with(';') { p += 1; }
        } else { break; }
    }
    let mut closes = String::new();
    let mut opens = String::new();
    for wp in wraps_at(&root, p) {
        if !splittable(&wp) { break; }
        closes.push_str(match wp.kind { WK::Star => "*", WK::Under => "_", WK::Func(..) => "]" });
        opens.insert_str(0, &w.text[wp.open.clone()]);
    }
    let eol = w.eol();
    let ins = format!("{closes}{eol}{eol}{opens}");
    w.caret = p; w.anchor = p;
    unchecked(w, p..p, &ins, true)?;
    w.anchor = w.caret;
    Ok(())
}

/// Shift+Enter: 같은 문단 안에서 줄만 바꾼다.
pub fn linebreak(w: &mut Work) -> R<()> {
    if w.sel().is_some() { delete_sel(w)?; }
    let node = tree(&w.text);
    let root = LinkedNode::new(&node);
    let p = w.caret;
    if !matches!(ctx(&root, p), Ctx::Markup) { return Err("이 자리에서는 줄을 바꿀 수 없습니다.".into()); }
    let r = checked(w, p..p, "\\ ");
    w.anchor = w.caret;
    r
}

/// Tab: 정해진 너비만큼 가로 간격을 넣는다.
pub fn tab(w: &mut Work, width_pt: f32) -> R<()> {
    if w.sel().is_some() { delete_sel(w)?; }
    let node = tree(&w.text);
    let root = LinkedNode::new(&node);
    let p = w.caret;
    if !matches!(ctx(&root, p), Ctx::Markup) { return Err("이 자리에는 탭을 넣을 수 없습니다.".into()); }
    let mut s = format!("#h({:.1}pt)", width_pt.max(1.0));
    if needs_guard(next_char(&w.text, p)) { s.push(';'); }
    let r = unchecked(w, p..p, &s, true);
    w.anchor = w.caret;
    r
}

/// 커서 앞에 붙어 있는 탭(`#h(..pt)`)들의 너비 합. 커서 모양을 그릴 때 쓴다.
pub fn tabs_between(text: &str, from: usize, to: usize) -> f32 {
    let Some(s) = text.get(from..to) else { return 0.0 };
    let mut sum = 0.0;
    let mut rest = s;
    while let Some(i) = rest.find("#h(") {
        let r = &rest[i + 3..];
        let Some(j) = r.find("pt)") else { break };
        if let Ok(v) = r[..j].parse::<f32>() { sum += v; }
        rest = &r[j + 3..];
    }
    sum
}

/// 내어쓰기(둘째 줄부터 들여 쓰기)를 1em씩 늘리거나 줄인다.
pub fn hanging(w: &mut Work, delta: i32) -> R<()> {
    let node = tree(&w.text);
    let root = LinkedNode::new(&node);
    let p = w.caret;
    if !matches!(ctx(&root, p), Ctx::Markup) { return Err("이 글에는 내어쓰기를 줄 수 없습니다.".into()); }
    // 이미 내어쓰기가 걸려 있는지
    if let Some(wp) = wraps_at(&root, p).into_iter().find(|x| x.is_hang(&w.text)) {
        let open = &w.text[wp.open.clone()];
        let i = open.find("hanging-indent").unwrap();
        let after = &open[i..];
        let c = after.find(':').ok_or("내어쓰기 값을 읽지 못했습니다.")? + 1;
        let val = after[c..].trim_start();
        let lead = after[c..].len() - val.len();
        let n = val.find("em").ok_or("내어쓰기 값을 읽지 못했습니다.")?;
        let cur: f32 = val[..n].trim().parse().map_err(|_| "내어쓰기 값을 읽지 못했습니다.".to_string())?;
        let new = cur + delta as f32;
        if new <= 0.01 {
            let mut end = wp.close.end;
            if w.text[end..].starts_with(';') { end += 1; }
            w.rep(wp.close.start..end, "", false);
            w.rep(wp.open.clone(), "", false);
        } else {
            let s = wp.open.start + i + c + lead;
            let txt = if (new - new.round()).abs() < 0.01 { format!("{}", new.round() as i32) } else { format!("{new:.1}") };
            w.rep(s..s + n, &txt, false);
        }
        if bad(&tree(&w.text)) { return Err("내어쓰기를 바꾸지 못했습니다.".into()); }
        return Ok(());
    }
    if delta < 0 { return Err("이 문단에는 내어쓰기가 없습니다.".into()); }
    let (s, e) = para_at(&root, &w.text, p).ok_or("빈 문단에는 내어쓰기를 줄 수 없습니다.")?;
    let close = if needs_guard(next_char(&w.text, e)) { "];" } else { "]" };
    w.rep(e..e, close, false);
    w.rep(s..s, "#par(hanging-indent: 1em)[", true);
    if bad(&tree(&w.text)) { return Err("이 문단에는 내어쓰기를 줄 수 없습니다.".into()); }
    Ok(())
}

/// 커서가 든 문단의 범위(문단 나눔 사이, 앞뒤 공백 제외).
fn para_at(root: &LinkedNode, text: &str, p: usize) -> Option<(usize, usize)> {
    let m = block_at(root, text, p)?;
    let mnode = find_node(root, &m, SyntaxKind::Markup)?;
    let (mut s, mut e) = (None::<usize>, None::<usize>);
    for c in mnode.children() {
        let r = c.range();
        if c.kind() == SyntaxKind::Parbreak {
            if r.end <= p { s = None; e = None; continue; }
            if r.start >= p { break; }
        }
        if c.kind() == SyntaxKind::Space { continue; }
        if s.is_none() { s = Some(r.start); }
        e = Some(r.end);
    }
    Some((s?, e?))
}

/// 세 번 누르기: 커서가 든 문단(따옴표 문자열 안이면 그 문자열 전체).
pub fn para_range(text: &str, p: usize) -> Option<(usize, usize)> {
    let node = tree(text);
    let root = LinkedNode::new(&node);
    if let Some(sr) = str_at(&root, p) { return Some((sr.start, sr.end)); }
    let (mut s, mut e) = para_at(&root, text, p)?;
    // 문단을 감싼 꾸밈(내어쓰기·정렬)이면 그 안쪽 글만
    loop {
        let Some(wp) = wraps_at(&root, s + 1).into_iter().find(|x| x.open.start == s && x.close.end >= e && x.is_parwrap(text)) else { break };
        let inner = text[wp.content.clone()].trim();
        if inner.is_empty() { break; }
        s = wp.content.start + text[wp.content.clone()].find(inner).unwrap_or(0);
        e = s + inner.len();
    }
    Some((s, e))
}

/// 원고의 `a`와 `b` 사이에 문단 나눔이 있고 본문 글자는 없는지(커서가 글자 없는 빈 문단에 있는지 가릴 때).
pub fn blank_gap(text: &str, a: usize, b: usize) -> bool {
    if a >= b || b > text.len() { return false; }
    let node = tree(text);
    let root = LinkedNode::new(&node);
    fn go(n: &LinkedNode, a: usize, b: usize, brk: &mut bool) -> bool {
        for c in n.children() {
            let r = c.range();
            if r.end <= a || r.start >= b { continue; }
            match c.kind() {
                SyntaxKind::Parbreak => *brk = true,
                SyntaxKind::Text | SyntaxKind::Escape | SyntaxKind::Shorthand | SyntaxKind::SmartQuote | SyntaxKind::Linebreak
                    if c.parent_kind() == Some(SyntaxKind::Markup) => return false,
                SyntaxKind::Equation | SyntaxKind::Raw => return false,
                _ => if !go(&c, a, b, brk) { return false; },
            }
        }
        true
    }
    let mut brk = false;
    go(&root, a, b, &mut brk) && brk
}

/// Ctrl+A: 커서가 든 글 상자 전체(앞뒤 공백 제외). 따옴표 문자열 안이면 그 문자열 전체.
pub fn block_range(text: &str, p: usize) -> Option<(usize, usize)> {
    let node = tree(text);
    let root = LinkedNode::new(&node);
    if let Some(sr) = str_at(&root, p) { return Some((sr.start, sr.end)); }
    let m = block_at(&root, text, p)?;
    let inner = text[m.clone()].trim();
    if inner.is_empty() { return None; }
    let s = m.start + text[m.clone()].find(inner).unwrap_or(0);
    Some((s, s + inner.len()))
}

/// 문단 정렬: `how`가 "center"·"right"·"left"면 문단을 `#align(..)[..]`로 감싸고, None이면 벗긴다(템플릿 기본 정렬).
pub fn align(w: &mut Work, how: Option<&str>) -> R<()> {
    let node = tree(&w.text);
    let root = LinkedNode::new(&node);
    let p = w.caret;
    if !matches!(ctx(&root, p), Ctx::Markup) { return Err("이 글에는 정렬을 줄 수 없습니다.".into()); }
    if let Some(wp) = wraps_at(&root, p).into_iter().find(|x| x.align_of(&w.text).is_some()) {
        let cur = wp.align_of(&w.text).unwrap().to_string();
        match how {
            Some(h) if h == cur => return Ok(()),
            Some(h) => {
                let s = wp.open.start + "#align(".len();
                w.rep(s..s + cur.len(), h, false);
            }
            None => {
                let mut end = wp.close.end;
                if w.text[end..].starts_with(';') { end += 1; }
                w.rep(wp.close.start..end, "", false);
                w.rep(wp.open.clone(), "", false);
            }
        }
        if bad(&tree(&w.text)) { return Err("정렬을 바꾸지 못했습니다.".into()); }
        return Ok(());
    }
    let Some(how) = how else { return Ok(()) };
    let (s, e) = para_at(&root, &w.text, p).ok_or("빈 문단에는 정렬을 줄 수 없습니다.")?;
    let close = if needs_guard(next_char(&w.text, e)) { "];" } else { "]" };
    w.rep(e..e, close, false);
    w.rep(s..s, &format!("#align({how})["), true);
    if bad(&tree(&w.text)) { return Err("이 문단에는 정렬을 줄 수 없습니다.".into()); }
    Ok(())
}

/// 커서가 든 문단에 걸린 정렬("center" 등). 없으면 None.
pub fn align_state(text: &str, p: usize) -> Option<String> {
    let node = tree(text);
    let root = LinkedNode::new(&node);
    wraps_at(&root, p).into_iter().find_map(|x| x.align_of(text).map(|s| s.to_string()))
}

// ───────────────────────── 굵게·밑줄

fn text_leaves(node: &LinkedNode, a: usize, b: usize, out: &mut Vec<Range<usize>>) {
    for c in node.children() {
        let r = c.range();
        if r.end <= a || r.start >= b { continue; }
        if is_markup_text(&c) { out.push(r); } else { text_leaves(&c, a, b, out); }
    }
}

pub const STYLES: [Style; 4] = [Style::Bold, Style::Italic, Style::Under, Style::Strike];

/// 꾸밈들(안쪽부터)이 그 서식을 주는지. 굵게는 가장 안쪽에서 굵기를 정한 것이 이긴다
/// (`*..*` 안의 `#text(weight: "regular")[..]`는 굵지 않다).
fn styled(ws: &[Wrap], st: Style) -> bool {
    if st == Style::Bold {
        for w in ws {
            if let Some(k) = &w.weight { return k.0; }
            if w.is_style(st) { return true; }
        }
        return false;
    }
    ws.iter().any(|w| w.is_style(st))
}

/// 글자 꾸밈 상태: [굵게, 기울임, 밑줄, 취소선].
pub fn fmt_state(text: &str, a: usize, b: usize) -> [bool; 4] {
    let node = tree(text);
    let root = LinkedNode::new(&node);
    let has = |p: usize, st: Style| styled(&wraps_at(&root, p), st);
    if a == b { return STYLES.map(|st| has(a, st)); }
    let mut leaves = vec![];
    text_leaves(&root, a, b, &mut leaves);
    if leaves.is_empty() { return [false; 4]; }
    let all = |st: Style| leaves.iter().all(|r| has(r.start.max(a) + 1, st) || has(r.start.max(a), st) && has(r.end.min(b), st));
    STYLES.map(all)
}

fn all_wraps(node: &LinkedNode, out: &mut Vec<Wrap>) {
    for c in node.children() {
        if let Some(w) = wrap_of(&c) { out.push(w); }
        all_wraps(&c, out);
    }
}

/// `p`가 Markup `m` 안의 글자 아닌 자식 속에 있으면 그 자식의 가장자리로 민다.
fn snap(root: &LinkedNode, m: &Range<usize>, p: usize, to_end: bool) -> usize {
    let Some(mn) = find_node(root, m, SyntaxKind::Markup) else { return p };
    for c in mn.children() {
        let r = c.range();
        if r.start < p && p < r.end && c.kind() != SyntaxKind::Text {
            if to_end { return r.end; }
            return c.prev_sibling_with_trivia().filter(|h| h.kind() == SyntaxKind::Hash).map(|h| h.range().start).unwrap_or(r.start);
        }
    }
    p
}

fn to_func(w: &mut Work, wp: &Wrap) {
    let name = if wp.kind == WK::Star { "#strong[" } else { "#emph[" };
    let close = if needs_guard(next_char(&w.text, wp.close.end)) { "];" } else { "]" };
    w.rep(wp.close.clone(), close, false);
    w.rep(wp.open.clone(), name, true);
}

fn remove_style(w: &mut Work, st: Style) -> R<()> { remove_wraps(w, &|x: &Wrap| x.is_style(st)) }

/// 선택에 걸친 꾸밈 가운데 `pick`이 고른 것을 걷어 낸다. 선택이 일부만 덮으면 꾸밈을 둘로 갈라 그 부분만 뺀다.
fn remove_wraps(w: &mut Work, pick: &dyn Fn(&Wrap) -> bool) -> R<()> {
    for _ in 0..200 {
        let (a, b) = match w.sel() { Some(x) => x, None => return Ok(()) };
        let node = tree(&w.text);
        let root = LinkedNode::new(&node);
        let mut ws = vec![];
        all_wraps(&root, &mut ws);
        let Some(wp) = ws.into_iter().find(|x| pick(x) && a.max(x.content.start) < b.min(x.content.end)) else { return Ok(()) };
        let (cs, ce) = (wp.content.start, wp.content.end);
        let os = snap(&root, &wp.content, a.max(cs), false);
        let oe = snap(&root, &wp.content, b.min(ce), true);
        // 갈라 낼 때 남는 꾸밈이 공백으로 끝나거나 시작하지 않게, 표식 자리를 공백 너머로 옮긴다
        // (`\ ` 줄 나눔 뒤의 공백은 그대로 둔다: 표식이 `\` 바로 뒤에 붙으면 글자로 읽힌다)
        let os = { let t = w.text[cs..os].trim_end(); if t.ends_with('\\') { os } else { cs + t.len() } };
        let oe = { let t = &w.text[oe..ce]; oe + (t.len() - t.trim_start().len()) };
        let func = matches!(wp.kind, WK::Func(..));
        let open_txt = w.text[wp.open.clone()].to_string();
        let close_txt = match wp.kind { WK::Func(..) => "]", WK::Star => "*", WK::Under => "_" };
        if !func {
            // *…* 꼴: 낱말 한가운데에 표식이 놓이면 깨지므로 그때는 함수 꼴로 바꿔 다시
            let risky = |p: usize| wordy(prev_char(&w.text, p)) && wordy(next_char(&w.text, p));
            if (os > cs && risky(os)) || (oe < ce && risky(oe)) { to_func(w, &wp); continue; }
        }
        let before = w.text.clone();
        let (c0, a0) = (w.caret, w.anchor);
        // 뒤쪽부터 고친다
        if oe < ce {
            let guard = if func && needs_guard(next_char(&w.text, oe)) { ";" } else { "" };
            let _ = guard;
            w.rep(oe..oe, &open_txt, true);
        } else {
            let mut end = wp.close.end;
            if func && w.text[end..].starts_with(';') { end += 1; }
            w.rep(wp.close.start..end, "", false);
        }
        if os > cs {
            let g = if func && needs_guard(next_char(&w.text, os)) { "];" } else { close_txt };
            w.rep(os..os, g, false);
        } else {
            w.rep(wp.open.clone(), "", false);
        }
        if bad(&tree(&w.text)) {
            w.text = before; w.caret = c0; w.anchor = a0;
            if !func { to_func(w, &wp); continue; }
            return Err("이 선택에는 서식을 바꿀 수 없습니다.".into());
        }
    }
    Ok(())
}

fn runs(node: &LinkedNode, a: usize, b: usize, out: &mut Vec<Range<usize>>) {
    let mut cur: Option<(usize, usize)> = None;
    let flush = |cur: &mut Option<(usize, usize)>, out: &mut Vec<Range<usize>>| {
        if let Some((s, e)) = cur.take() { if e > s { out.push(s..e); } }
    };
    for c in node.children() {
        let r = c.range();
        if r.end <= a || r.start >= b { continue; }
        let whole = a <= r.start && r.end <= b;
        let k = c.kind();
        if k == SyntaxKind::Parbreak { flush(&mut cur, out); continue; }
        if whole || k == SyntaxKind::Text || k == SyntaxKind::Space {
            if k == SyntaxKind::Space { continue; }
            let (s, e) = (a.max(r.start), b.min(r.end));
            // 식 뒤의 `;`까지 한 덩어리로
            cur = Some(match cur { Some((cs, _)) => (cs, e), None => (s, e) });
        } else {
            flush(&mut cur, out);
            fn down(n: &LinkedNode, a: usize, b: usize, out: &mut Vec<Range<usize>>) {
                for c in n.children() {
                    let r = c.range();
                    if r.end <= a || r.start >= b { continue; }
                    if c.kind() == SyntaxKind::Markup { runs(&c, a, b, out); } else { down(&c, a, b, out); }
                }
            }
            down(&c, a, b, out);
        }
    }
    flush(&mut cur, out);
}

fn add_style(w: &mut Work, st: Style, func_open: &str) -> R<()> {
    // 굵게·기울임은 되도록 짧은 표식(`*`, `_`)으로, 낱말 한가운데 등 표식이 깨지는 자리에서는 함수 꼴로
    add_wrap(w, match st { Style::Bold => Some("*"), Style::Italic => Some("_"), _ => None }, func_open)
}

fn add_wrap(w: &mut Work, mark: Option<&str>, func_open: &str) -> R<()> {
    let Some((a, b)) = w.sel() else { return Ok(()) };
    let node = tree(&w.text);
    let root = LinkedNode::new(&node);
    let mut rs = vec![];
    runs(&root, a, b, &mut rs);
    // 앞뒤 공백은 꾸밈 밖에 둔다
    let rs: Vec<Range<usize>> = rs.into_iter().filter_map(|r| {
        let t = &w.text[r.clone()];
        let s = r.start + (t.len() - t.trim_start().len());
        let e = r.end - (t.len() - t.trim_end().len());
        if e > s { Some(s..e) } else { None }
    }).collect();
    if rs.is_empty() { return Err("서식을 줄 글자가 없습니다.".into()); }
    let before = w.text.clone();
    let (c0, a0) = (w.caret, w.anchor);
    for pass in 0..2 {
        let mut sorted = rs.clone();
        sorted.sort_by_key(|r| std::cmp::Reverse(r.start));
        for r in sorted {
            let t = &w.text;
            let mark_ok = mark.is_some() && pass == 0
                && !(wordy(prev_char(t, r.start)) && wordy(next_char(t, r.start)))
                && !(wordy(prev_char(t, r.end)) && wordy(next_char(t, r.end)))
                && prev_char(t, r.start) != Some('\\')
                && !t[r.clone()].contains(mark.unwrap());
            let (open, close) = if mark_ok { (mark.unwrap().to_string(), mark.unwrap().to_string()) } else {
                (func_open.to_string(), if needs_guard(next_char(t, r.end)) { "];".to_string() } else { "]".to_string() })
            };
            w.rep(r.end..r.end, &close, false);
            w.rep(r.start..r.start, &open, true);
        }
        if !bad(&tree(&w.text)) { return Ok(()); }
        w.text = before.clone(); w.caret = c0; w.anchor = a0;
    }
    Err("이 선택에는 서식을 줄 수 없습니다.".into())
}

/// 서식을 바꿀 선택 범위. 표시를 선택 범위로 맞춘다(시작 = anchor, 끝 = caret).
fn style_sel(w: &mut Work) -> R<(usize, usize)> {
    let Some((a, b)) = w.sel() else { return Err("먼저 서식을 바꿀 글자를 끌어서 선택해 주세요.".into()) };
    let node = tree(&w.text);
    let root = LinkedNode::new(&node);
    if str_at(&root, a).is_some() || str_at(&root, b).is_some() {
        return Err("제목처럼 따옴표 안에 든 글에는 굵게·밑줄 같은 꾸밈을 줄 수 없습니다.".into());
    }
    w.anchor = a; w.caret = b;
    Ok((a, b))
}

/// 선택에 걸친 `#text(weight: ..)` 굵기를 걷어 낸다. `bold`: 굵게 만드는 것(true)인지, 굵기를 푸는 것(false)인지.
fn strip_weight(w: &mut Work, bold: bool) -> R<()> {
    // 크기·색 같은 다른 인자와 함께 쓰였으면: 선택이 내용을 통째로 덮었을 때만 굵기 인자를 지운다
    for _ in 0..50 {
        let Some((a, b)) = w.sel() else { return Ok(()) };
        let node = tree(&w.text);
        let root = LinkedNode::new(&node);
        let mut ws = vec![];
        all_wraps(&root, &mut ws);
        let hit = ws.into_iter().find_map(|x| {
            let (k, r, solo) = x.weight.clone()?;
            if k != bold || solo { return None; }
            let t = &w.text[x.content.clone()];
            let cs = x.content.start + (t.len() - t.trim_start().len());
            let ce = x.content.end - (t.len() - t.trim_end().len());
            (cs < ce && a <= cs && ce <= b).then_some(r)
        });
        let Some(r) = hit else { break };
        let (mut s, mut e) = (r.start, r.end);
        let rest = w.text[e..].trim_start_matches(' ');
        if rest.starts_with(',') {
            e = w.text.len() - rest.len() + 1;
            e += w.text[e..].len() - w.text[e..].trim_start_matches(' ').len();
        } else {
            let head = w.text[..s].trim_end();
            if head.ends_with(',') { s = head.len() - 1; }
        }
        let before = w.text.clone();
        let (c0, a0) = (w.caret, w.anchor);
        w.rep(s..e, "", false);
        if bad(&tree(&w.text)) { w.text = before; w.caret = c0; w.anchor = a0; break; }
    }
    // 굵기만 준 `#text(weight: ..)[..]`는 통째로(선택이 일부면 그 부분만) 걷어 낸다
    remove_wraps(w, &|x: &Wrap| x.weight.as_ref().is_some_and(|k| k.0 == bold && k.2))
}

/// 굵게 바꾸기의 첫 단계: 지금 굵기를 만든 꾸밈을 원고에서 걷어 낸다.
/// `on`(지금 굵다 → 끈다): `*..*`·`#strong[..]`·`#text(weight: "bold")`를, 아니면 굵기를 풀던 `#text(weight: "regular")[..]`를.
pub fn bold_strip(w: &mut Work, on: bool) -> R<()> {
    style_sel(w)?;
    if on { remove_style(w, Style::Bold)?; }
    strip_weight(w, on)
}

/// 둘째 단계: 걷어 낼 것이 없거나 걷어 내도 그대로일 때(템플릿·show 규칙이 준 굵기) 꾸밈을 덧씌운다.
pub fn bold_wrap(w: &mut Work, on: bool) -> R<()> {
    if on { return add_wrap(w, None, "#text(weight: \"regular\")["); }
    remove_style(w, Style::Bold)?;
    add_style(w, Style::Bold, "#strong[")
}

/// 서식 표식만 걷어 낸다(밑줄·취소선 풀기의 첫 단계).
pub fn style_strip(w: &mut Work, st: Style) -> R<()> {
    style_sel(w)?;
    remove_style(w, st)
}

/// 선택을 `open`으로 여는 함수 꾸밈으로 감싼다.
pub fn wrap_with(w: &mut Work, open: &str) -> R<()> { add_wrap(w, None, open) }

/// 벗겨 볼 수 있는 글줄 꾸밈의 종류: `*..*`·`_.._`와, 인자 없이 글줄에 낀 `#이름[..]`.
fn peel_key(x: &Wrap) -> Option<String> {
    match &x.kind {
        WK::Star => Some("strong".into()),
        WK::Under => Some("emph".into()),
        WK::Func(n, false) if x.inline() || !x.blocky => Some(n.clone()),
        _ => None,
    }
}

/// 선택에 걸친 글줄 꾸밈의 종류(안쪽부터). 문서의 규칙이나 템플릿이 이 꾸밈에 밑줄 같은 것을 입혔는지
/// 하나씩 벗겨 보며 찾을 때의 후보다.
pub fn peel_kinds(text: &str, a: usize, b: usize) -> Vec<String> {
    let node = tree(text);
    let root = LinkedNode::new(&node);
    let mut ws = vec![];
    all_wraps(&root, &mut ws);
    ws.retain(|x| a.max(x.content.start) < b.min(x.content.end));
    ws.sort_by_key(|x| x.content.len());
    let mut out: Vec<String> = vec![];
    for x in &ws {
        if let Some(k) = peel_key(x) { if !out.contains(&k) { out.push(k); } }
    }
    out
}

/// 선택에 걸친 `key` 종류의 꾸밈을 벗긴다(선택이 일부만 덮으면 그 부분만).
pub fn peel(w: &mut Work, key: &str) -> R<()> {
    style_sel(w)?;
    remove_wraps(w, &|x: &Wrap| peel_key(x).as_deref() == Some(key))
}

const COLOR_NAMES: &[&str] = &["black", "gray", "silver", "white", "navy", "blue", "aqua", "teal", "eastern", "purple", "fuchsia",
    "maroon", "red", "orange", "yellow", "olive", "green", "lime"];

/// 색을 나타내는 식인지: 색 이름, `rgb(..)`·`luma(..)` 같은 부름, 또는 그런 값으로 정해 둔 변수.
fn colorish(root: &LinkedNode, text: &str, c: &LinkedNode) -> bool {
    let is_call = |t: &str| ["rgb(", "luma(", "cmyk(", "oklab(", "oklch(", "color."].iter().any(|p| t.trim_start().starts_with(p));
    let t = &text[c.range()];
    match c.kind() {
        SyntaxKind::FuncCall | SyntaxKind::FieldAccess => is_call(t) || COLOR_NAMES.iter().any(|n| t.starts_with(&format!("{n}."))),
        SyntaxKind::Ident => COLOR_NAMES.contains(&t) || let_value(root, t, usize::MAX).is_some_and(|r| is_call(&text[r])),
        _ => false,
    }
}

/// `#text(..)`에서 크기(`size`)나 색(`fill`)을 정한 인자의 범위: 이름 붙은 인자, 또는 자리로 준 값(`#text(9pt, navy)[..]`).
/// 돌려주는 것: (값의 범위, 인자 전체의 범위).
fn text_slot(root: &LinkedNode, text: &str, f: &LinkedNode, key: &str) -> Option<(Range<usize>, Range<usize>)> {
    let args = f.children().find(|c| c.kind() == SyntaxKind::Args)?;
    for c in args.children() {
        match c.kind() {
            SyntaxKind::Named => if c.children().next().is_some_and(|k| k.get().full_text() == key) {
                return c.children().last().map(|v| (v.range(), c.range()));
            },
            SyntaxKind::Numeric => if key == "size" { return Some((c.range(), c.range())); },
            SyntaxKind::Ident | SyntaxKind::FuncCall | SyntaxKind::FieldAccess => if key == "fill" && colorish(root, text, &c) { return Some((c.range(), c.range())); },
            _ => {}
        }
    }
    None
}

/// 선택한 글의 글자 크기(`size`)나 색(`fill`)을 정한다. `calc`: 지금 적힌 값(없으면 None) → 새 값(None이면 그 속성을 지운다).
/// 선택이 `#text(..)[..]`의 내용 전체면 그 인자를 고치고, 아니면 `#text(key: 값)[..]`으로 감싼다.
fn text_prop(w: &mut Work, key: &str, calc: &dyn Fn(Option<&str>) -> R<Option<String>>) -> R<()> {
    let (a, b) = style_sel(w)?;
    let before = w.text.clone();
    let (c0, a0) = (w.caret, w.anchor);
    let node = tree(&w.text);
    let root = LinkedNode::new(&node);
    let trim = |s: usize, e: usize| { let t = &before[s..e]; (s + (t.len() - t.trim_start().len()), e - (t.len() - t.trim_end().len())) };
    let (ta, tb) = trim(a, b);
    let mut ws = vec![];
    all_wraps(&root, &mut ws);
    // 선택이 내용을 통째로 덮은 `#text(..)[..]`: 그 속성을 이미 가진 것이 먼저, 다음은 가장 안쪽
    let mut hits: Vec<(bool, Wrap, Option<(Range<usize>, Range<usize>)>)> = vec![];
    for x in ws {
        if !matches!(&x.kind, WK::Func(n, _) if n == "text") { continue; }
        let (cs, ce) = trim(x.content.start, x.content.end);
        if !(cs < ce && ta >= x.content.start && tb <= x.content.end && ta <= cs && tb >= ce) { continue; }
        let Some(f) = find_node(&root, &x.node, SyntaxKind::FuncCall) else { continue };
        let slot = text_slot(&root, &before, &f, key);
        hits.push((slot.is_some(), x, slot));
    }
    hits.sort_by_key(|h| (!h.0, h.1.content.len()));
    match hits.into_iter().next() {
        Some((_, x, slot)) => {
            let new = calc(slot.as_ref().map(|r| &before[r.0.clone()]))?;
            match (slot, new) {
                (Some((r, _)), Some(v)) => w.rep(r, &v, false),
                (Some((_, r)), None) => {
                    // 그 인자를 지운다. 남는 인자가 없으면 꾸밈째로 걷어 낸다
                    let (mut s, mut e) = (r.start, r.end);
                    let rest = before[e..].trim_start_matches(' ');
                    if rest.starts_with(',') {
                        e = before.len() - rest.len() + 1;
                        e += before[e..].len() - before[e..].trim_start_matches(' ').len();
                    } else {
                        let head = before[..s].trim_end();
                        if head.ends_with(',') { s = head.len() - 1; }
                    }
                    let open = format!("{}{}", &before[x.open.start..s], &before[e..x.open.end]);
                    if open.replace(' ', "") == "#text()[" {
                        let mut end = x.close.end;
                        if before[end..].starts_with(';') { end += 1; }
                        w.rep(x.close.start..end, "", false);
                        w.rep(x.open.clone(), "", false);
                    } else {
                        w.rep(s..e, "", false);
                    }
                }
                (None, Some(v)) => {
                    let f = find_node(&root, &x.node, SyntaxKind::FuncCall).ok_or(NO_ELEM)?;
                    let (at, ins) = arg_insert(&f, key, &v).ok_or(NO_ELEM)?;
                    w.rep(at, &ins, false);
                }
                (None, None) => {}
            }
        }
        None => {
            if let Some(v) = calc(None)? { add_wrap(w, None, &format!("#text({key}: {v})["))?; }
        }
    }
    if bad(&tree(&w.text)) {
        w.text = before; w.caret = c0; w.anchor = a0;
        return Err("이 선택에는 서식을 바꿀 수 없습니다.".into());
    }
    Ok(())
}

/// 선택한 글의 글자 크기를 `new`pt로. `cur`: 지금 지면에서의 크기.
pub fn text_size(w: &mut Work, cur: f32, new: f32) -> R<()> {
    text_prop(w, "size", &|old| Ok(Some(match old {
        Some(s) => set_len(s, Some(cur), new - cur)?,
        None => format!("{}pt", fmt_num(new, 1)),
    })))
}

/// 선택한 글의 글자 색을 정한다(`#rrggbb`). None이면 따로 준 색을 지운다.
pub fn text_fill(w: &mut Work, hex: Option<&str>) -> R<()> {
    text_prop(w, "fill", &|_| Ok(hex.map(|h| format!("rgb(\"{h}\")"))))
}

/// 굵게·기울임·밑줄·취소선을 켜거나 끈다. `under_fn`: 밑줄에 쓸 함수 이름(책의 템플릿에 `u`가 있으면 그것).
/// 여러 글 상자에 걸친 선택이면 상자마다 따로 꾸민다.
pub fn toggle(w: &mut Work, st: Style, under_fn: &str) -> R<()> {
    let (a, b) = style_sel(w)?;
    let on = fmt_state(&w.text, a, b)[STYLES.iter().position(|x| *x == st).unwrap()];
    if st == Style::Bold {
        let before = w.text.clone();
        bold_strip(w, on)?;
        if w.text == before { bold_wrap(w, on)?; }
        return Ok(());
    }
    remove_style(w, st)?;
    if !on {
        let open = match st {
            Style::Bold => "#strong[".to_string(),
            Style::Italic => "#emph[".to_string(),
            Style::Under => format!("#{under_fn}["),
            Style::Strike => "#strike[".to_string(),
        };
        add_style(w, st, &open)?;
    }
    Ok(())
}

/// 낱말 범위(두 번 누르기).
pub fn word_at(text: &str, p: usize) -> (usize, usize) {
    let is_w = |c: char| c.is_alphanumeric() || c == '\'' || c == '’';
    let mut s = p;
    while let Some(c) = text[..s].chars().next_back() { if is_w(c) { s -= c.len_utf8(); } else { break; } }
    let mut e = p;
    while let Some(c) = text[e..].chars().next() { if is_w(c) { e += c.len_utf8(); } else { break; } }
    (s, e)
}

// ───────────────────────── 요소(함수 부름) 고르기·지우기·옮기기

pub struct Elem { pub range: Range<usize>, pub name: String }

/// `pos`의 글자를 만들어 내는 함수 부름들(안쪽부터). 함수 정의의 경계에서 멈춘다.
pub fn elem_chain(text: &str, pos: usize) -> Vec<Elem> {
    let node = tree(text);
    let root = LinkedNode::new(&node);
    let Some(leaf) = root.leaf_at(pos, Side::After).or_else(|| root.leaf_at(pos, Side::Before)) else { return vec![] };
    let mut out = vec![];
    let mut n = leaf;
    loop {
        match n.kind() {
            SyntaxKind::FuncCall => out.push(Elem { range: n.range(), name: callee_name(&n) }),
            SyntaxKind::Closure => break,
            _ => {}
        }
        match n.parent() { Some(p) => n = p.clone(), None => break }
    }
    out
}

const NO_ELEM: &str = "요소를 찾지 못했습니다. 다시 골라 주세요.";

/// 요소를 지운다. 쓰인 자리에 따라 통째로 지우거나 빈 값으로 바꾼다.
pub fn elem_delete(text: &str, r: &Range<usize>) -> R<String> {
    let node = tree(text);
    let root = LinkedNode::new(&node);
    let f = find_node(&root, r, SyntaxKind::FuncCall).ok_or(NO_ELEM)?;
    let mut t = text.to_string();
    let whole_line = |t: &str, r: &Range<usize>| -> Range<usize> {
        let ls = t[..r.start].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let le = t[r.end..].find('\n').map(|i| r.end + i + 1).unwrap_or(t.len());
        if t[ls..r.start].trim().is_empty() && t[r.end..le].trim().is_empty() { ls..le } else { r.clone() }
    };
    if let Some(h) = hash_start(&f) {
        let mut end = r.end;
        if t[end..].starts_with(';') { end += 1; }
        let d = whole_line(&t, &(h..end));
        t.replace_range(d, "");
    } else {
        match f.parent_kind() {
            Some(SyntaxKind::Named) => t.replace_range(r.clone(), "none"),
            Some(SyntaxKind::Args) | Some(SyntaxKind::Array) if item_removable(&root, &f) => t.replace_range(item_range(text, &f), ""),
            Some(SyntaxKind::Args) => t.replace_range(r.clone(), "[]"),
            Some(SyntaxKind::Code) | Some(SyntaxKind::CodeBlock) => { let d = whole_line(&t, r); t.replace_range(d, ""); }
            _ => t.replace_range(r.clone(), "none"),
        }
    }
    if bad(&tree(&t)) { return Err("이 요소는 지면에서 지울 수 없습니다.".into()); }
    Ok(t)
}

fn fmt_pt(v: f32) -> String {
    let v = (v * 10.0).round() / 10.0;
    if v == 0.0 { "0pt".into() } else if v.fract() == 0.0 { format!("{}pt", v as i64) } else { format!("{v:.1}pt") }
}

/// 길이 값에 `delta`(pt)를 더한 새 표기.
fn bump_len(val: &str, delta: f32) -> String {
    let v = val.trim();
    for (unit, per) in [("mm", 2.834_645_7f32), ("cm", 28.346_457), ("in", 72.0), ("pt", 1.0)] {
        if let Some(num) = v.strip_suffix(unit) {
            if let Ok(n) = num.trim().parse::<f32>() {
                let x = ((n + delta / per) * 100.0).round() / 100.0;
                let s = if x.fract() == 0.0 { format!("{}", x as i64) } else { format!("{x}") };
                return format!("{s}{unit}");
            }
        }
    }
    // `식 + 3pt` 꼴이면 끝의 수만 고친다
    if let Some(i) = v.rfind(" + ") {
        if let Some(num) = v[i + 3..].strip_suffix("pt") {
            if let Ok(n) = num.parse::<f32>() { return format!("{} + {}", &v[..i], fmt_pt(n + delta)); }
        }
    }
    format!("{v} + {}", fmt_pt(delta))
}

const BLOCKY: &[&str] = &[
    "v", "block", "line", "place", "grid", "pagebreak", "align", "pad", "stack", "table", "figure", "par", "rect",
    "colbreak", "heading", "move", "columns", "image", "hide",
];

/// 요소가 제 문단을 혼자 차지하는지(앞뒤가 세로 간격·블록이거나 비어 있는지).
fn block_like(f: &LinkedNode, name: &str) -> bool {
    if BLOCKY.contains(&name) { return true; }
    let blocky = |n: &LinkedNode| match n.kind() {
        SyntaxKind::FuncCall => BLOCKY.contains(&callee_name(n).as_str()),
        SyntaxKind::Parbreak | SyntaxKind::LetBinding | SyntaxKind::SetRule | SyntaxKind::ShowRule => true,
        _ => false,
    };
    if hash_start(f).is_some() {
        // 마크업: 여러 문단을 담았으면 덩어리, 아니면 같은 문단에 함께 흐르는 글이 없어야 덩어리
        let multi = f.children().filter(|c| c.kind() == SyntaxKind::Args)
            .flat_map(|a| a.children().collect::<Vec<_>>())
            .filter(|c| c.kind() == SyntaxKind::ContentBlock)
            .flat_map(|cb| cb.children().collect::<Vec<_>>())
            .any(|m| m.kind() == SyntaxKind::Markup && m.children().any(|x| x.kind() == SyntaxKind::Parbreak));
        return multi || !(company(f, false) || company(f, true));
    }
    match f.parent_kind() {
        Some(SyntaxKind::Code) => f.prev_sibling().map_or(true, |p| blocky(&p)) && f.next_sibling().map_or(true, |n| blocky(&n)),
        Some(SyntaxKind::Args) => f.parent().and_then(|a| a.parent().cloned()).is_some_and(|c| BLOCKY.contains(&callee_name(&c).as_str())),
        _ => false,
    }
}

/// 마크업의 `#요소` 한쪽 옆(같은 문단)에 함께 흐르는 글이 있는지.
/// 글자·굵게 따위가 있으면 그렇고, 다른 `#요소`는 원고의 같은 줄에 나란히 있을 때만 그렇게 본다
/// (`#제목(..)` 다음 줄의 `#본문[..]`처럼 줄을 달리한 요소들은 저마다 따로 선 덩어리).
fn company(f: &LinkedNode, fwd: bool) -> bool {
    fn side<'a>(n: &LinkedNode<'a>, fwd: bool) -> Option<LinkedNode<'a>> {
        if fwd { n.next_sibling_with_trivia() } else { n.prev_sibling_with_trivia() }
    }
    // 뒤로 갈 때는 이 요소의 `#`부터 건넌다
    let mut cur = if fwd { side(f, fwd) } else { f.prev_sibling_with_trivia().and_then(|h| side(&h, fwd)) };
    let mut newline = false;
    while let Some(x) = cur {
        match x.kind() {
            SyntaxKind::Space => { if x.get().full_text().contains('\n') { newline = true; } }
            SyntaxKind::Parbreak => return false,
            SyntaxKind::LineComment | SyntaxKind::BlockComment | SyntaxKind::Semicolon => {}
            SyntaxKind::Text | SyntaxKind::Strong | SyntaxKind::Emph | SyntaxKind::Escape | SyntaxKind::Shorthand
            | SyntaxKind::SmartQuote | SyntaxKind::Linebreak | SyntaxKind::Link | SyntaxKind::Raw | SyntaxKind::Equation => return true,
            SyntaxKind::Hash if fwd => return !newline,
            SyntaxKind::Hash => {}
            // 다른 `#요소`(뒤로 갈 때는 요소가 `#`보다 먼저 나온다)
            _ => return !newline,
        }
        cur = side(&x, fwd);
    }
    false
}


/// 요소를 (dx, dy)pt 만큼 옮긴다. `place`·`move`면 그 값을 고치고, 아니면 `move(..)`로 감싼다.
/// 돌려주는 범위는 옮긴 뒤에 골라 둘 요소.
pub fn elem_move(text: &str, r: &Range<usize>, dx: f32, dy: f32) -> R<(String, Range<usize>)> {
    let node = tree(text);
    let root = LinkedNode::new(&node);
    let f = find_node(&root, r, SyntaxKind::FuncCall).ok_or(NO_ELEM)?;
    let name = callee_name(&f);
    let mut t = text.to_string();
    let out_range;
    if name == "place" || name == "move" {
        let args = f.children().find(|c| c.kind() == SyntaxKind::Args).ok_or(NO_ELEM)?;
        let mut edits: Vec<(Range<usize>, String)> = vec![];
        let mut missing = String::new();
        for (key, d) in [("dx", dx), ("dy", dy)] {
            if d == 0.0 { continue; }
            let named = args.children().find(|c| c.kind() == SyntaxKind::Named
                && c.children().next().is_some_and(|k| k.get().full_text() == key));
            match named {
                Some(n) => {
                    let val = n.children().last().ok_or(NO_ELEM)?;
                    edits.push((val.range(), bump_len(&text[val.range()], d)));
                }
                None => missing.push_str(&format!("{key}: {}, ", fmt_pt(d))),
            }
        }
        if !missing.is_empty() {
            match args.children().next() {
                Some(lp) if lp.kind() == SyntaxKind::LeftParen => edits.push((lp.range().end..lp.range().end, missing)),
                _ => { let s = args.range().start; edits.push((s..s, format!("({})", missing.trim_end_matches(", ")))); }
            }
        }
        edits.sort_by_key(|e| std::cmp::Reverse(e.0.start));
        let mut end = r.end as i64;
        for (er, s) in edits { end += s.len() as i64 - er.len() as i64; t.replace_range(er, &s); }
        out_range = r.start..end as usize;
    } else {
        // 이미 옮겨 둔 요소(`let it = ..; place(.., it); hide(it)`)의 알맹이를 골랐으면 그 place를 고친다
        if f.parent_kind() == Some(SyntaxKind::LetBinding) {
            if let Some(code) = f.parent().and_then(|l| l.parent()) {
                let ct = &text[code.range()];
                if ct.contains("hide(it)") {
                    if let Some(pl) = code.children().find(|c| c.kind() == SyntaxKind::FuncCall && callee_name(c) == "place") {
                        return elem_move(text, &pl.range(), dx, dy);
                    }
                }
            }
        }
        let inner = &text[r.clone()];
        if block_like(&f, &name) {
            // 제자리에는 보이지 않는 사본을 두어 다른 글이 밀리지 않게 하고, 보이는 것만 옮겨 놓는다
            let pre = format!("{{ let it = {inner}; ");
            let pl = format!("place(dx: {}, dy: {}, it)", fmt_pt(dx), fmt_pt(dy));
            out_range = r.start + pre.len()..r.start + pre.len() + pl.len();
            t.replace_range(r.clone(), &format!("{pre}{pl}; hide(it) }}"));
        } else {
            // 글줄 안에 낀 요소: 줄을 끊지 않도록 box(move(..))로 감싼다
            let head = format!("move(dx: {}, dy: {}", fmt_pt(dx), fmt_pt(dy));
            let new = if hash_start(&f).is_some() { format!("box({head})[#{inner}])") } else { format!("box({head}, {inner}))") };
            out_range = r.start + 4..r.start + new.len() - 1;
            t.replace_range(r.clone(), &new);
        }
    }
    if bad(&tree(&t)) { return Err("이 요소는 지면에서 옮길 수 없습니다.".into()); }
    Ok((t, out_range))
}

// ───────────────────────── 상자·표의 너비

/// 너비를 지면에서 끌어 바꿀 수 있는 상자 요소.
const SIZED: &[&str] = &["block", "box", "rect", "square", "ellipse", "circle", "image"];
const PT_PER: [(&str, f32); 4] = [("mm", 2.834_645_7), ("cm", 28.346_457), ("in", 72.0), ("pt", 1.0)];

/// 요소가 너비를 바꿀 수 있는 것이면 (이름, 표·격자인지).
pub fn resize_kind(text: &str, r: &Range<usize>) -> Option<(String, bool)> {
    let node = tree(text);
    let root = LinkedNode::new(&node);
    let f = find_node(&root, r, SyntaxKind::FuncCall)?;
    let name = callee_name(&f);
    if name == "grid" || name == "table" { return Some((name, true)); }
    SIZED.contains(&name.as_str()).then_some((name, false))
}

enum Len { Abs(f32, &'static str, f32), Rel(f32, &'static str), Fr(f32), Auto, Other }

fn parse_len(v: &str) -> Len {
    let v = v.trim();
    if v == "auto" { return Len::Auto; }
    let num = |unit: &str| v.strip_suffix(unit).and_then(|n| n.trim_end().parse::<f32>().ok());
    for (unit, per) in PT_PER { if let Some(n) = num(unit) { return Len::Abs(n, unit, per); } }
    if let Some(n) = num("em") { return Len::Rel(n, "em"); }
    if let Some(n) = num("%") { return Len::Rel(n, "%"); }
    if let Some(n) = num("fr") { return Len::Fr(n); }
    Len::Other
}

/// 소수 `digits`자리까지, 끝의 0은 떼고.
fn fmt_num(x: f32, digits: usize) -> String {
    let s = format!("{x:.digits$}");
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    if s == "-0" { "0".into() } else { s }
}
fn digits_of(unit: &str) -> usize { match unit { "mm" | "pt" | "%" => 1, _ => 2 } }
fn abs_mm(pt: f32) -> String { format!("{}mm", fmt_num(pt / 2.834_645_7, 1)) }

/// 길이 표기를 새 너비에 맞게 고친다. `cur`: 지금 지면에서의 너비(pt, 모르면 None), `delta`: 늘릴 양(pt).
/// 원고에 적힌 단위를 지킨다. `auto`·`fr`처럼 수로 정해지지 않는 값은 mm로 바꿔 적는다.
fn set_len(spec: &str, cur: Option<f32>, delta: f32) -> R<String> {
    const NEED: &str = "지금 너비를 지면에서 알아내지 못해 이 값은 바꿀 수 없습니다.";
    match parse_len(spec) {
        Len::Abs(n, unit, per) => Ok(format!("{}{unit}", fmt_num((n + delta / per).max(0.0), digits_of(unit)))),
        Len::Rel(n, unit) => {
            let c = cur.filter(|c| *c > 0.5).ok_or(NEED)?;
            Ok(format!("{}{unit}", fmt_num(n * ((c + delta).max(0.0) / c), digits_of(unit))))
        }
        Len::Fr(_) | Len::Auto => Ok(abs_mm((cur.ok_or(NEED)? + delta).max(0.0))),
        Len::Other => Ok(bump_len(spec, delta)),
    }
}

const NO_COLS: &str = "이 표의 칸 너비는 원고에 수로 적혀 있지 않아(변수나 식) 지면에서 바꿀 수 없습니다.";
const FR_EDGE: &str = "이 표에는 남는 너비를 채우는 칸(fr)이 있어 전체 너비가 표를 담은 자리에 맞춰집니다. 칸 사이의 경계를 끌어 주세요.";
const MIN_W: f32 = 6.0;

const NO_ROWS: &str = "이 표의 줄 높이는 원고에 수로 적혀 있지 않아(변수나 식) 지면에서 바꿀 수 없습니다.";
const MIN_H: f32 = 3.0;

/// 상자·표의 너비를 `dx`pt 만큼 바꾼다.
/// `col`: None이면 오른쪽 바깥 가장자리, Some(i)면 i번째 칸과 그다음 칸 사이의 경계.
/// `width`: 지금 지면에서의 전체 너비, `cols`: 지금 칸마다의 너비(표·격자).
pub fn elem_resize(text: &str, r: &Range<usize>, col: Option<usize>, dx: f32, width: Option<f32>, cols: &[f32]) -> R<String> {
    resize_ax(text, r, false, col, dx, width, cols)
}

/// 너비(`vert` 아님)나 높이(`vert`)를 `dx`pt 만큼 바꾼다. 높이일 때:
/// `col`은 줄 번호(그 줄의 아래 경계를 끈다. None이면 맨 아래), `width`는 전체 높이, `cols`는 줄마다의 높이.
/// 줄은 칸과 달리 끈 줄만 바뀌고 아래 줄들은 그만큼 밀린다.
fn resize_ax(text: &str, r: &Range<usize>, vert: bool, col: Option<usize>, dx: f32, width: Option<f32>, cols: &[f32]) -> R<String> {
    let node = tree(text);
    let root = LinkedNode::new(&node);
    let f = find_node(&root, r, SyntaxKind::FuncCall).ok_or(NO_ELEM)?;
    let name = callee_name(&f);
    let args = f.children().find(|c| c.kind() == SyntaxKind::Args).ok_or(NO_ELEM)?;
    let named = |key: &str| args.children().find(|c| c.kind() == SyntaxKind::Named
        && c.children().next().is_some_and(|k| k.get().full_text() == key));
    let (keyc, keyw, no) = if vert { ("rows", "height", NO_ROWS) } else { ("columns", "width", NO_COLS) };
    let mut t = text.to_string();
    if name == "grid" || name == "table" {
        let val = named(keyc).and_then(|n| n.children().last());
        if val.is_none() && !vert { return Err(no.into()); }
        // 칸(줄)마다의 표기와 원고 자리(배열이 아니면 값 전체를 새 배열로 바꿔 쓴다)
        let mut items: Vec<(Option<Range<usize>>, String)> = vec![];
        if let Some(val) = &val {
            match val.kind() {
                SyntaxKind::Array => for c in val.children() {
                    match c.kind() {
                        SyntaxKind::LeftParen | SyntaxKind::RightParen | SyntaxKind::Comma | SyntaxKind::Space
                        | SyntaxKind::LineComment | SyntaxKind::BlockComment => {}
                        SyntaxKind::Spread | SyntaxKind::Named | SyntaxKind::Keyed => return Err(no.into()),
                        _ => items.push((Some(c.range()), text[c.range()].to_string())),
                    }
                },
                SyntaxKind::Int => {
                    let n: usize = text[val.range()].trim().parse().map_err(|_| no.to_string())?;
                    items = vec![(None, "auto".to_string()); n];
                }
                _ => items.push((None, text[val.range()].to_string())),
            }
        }
        if vert {
            // 줄 수는 지면에서 센 것을 따른다: 적힌 값이 모자라면 마지막 값이 되풀이되고(Typst의 규칙), 없으면 auto
            let n = cols.len();
            if n == 0 { return Err(no.into()); }
            if items.len() != n {
                let last = items.last().map(|x| x.1.clone()).unwrap_or_else(|| "auto".into());
                let mut v: Vec<(Option<Range<usize>>, String)> = items.iter().take(n).map(|x| (None, x.1.clone())).collect();
                while v.len() < n { v.push((None, last.clone())); }
                items = v;
            }
        }
        let n = items.len();
        if n == 0 || items.iter().any(|x| matches!(parse_len(&x.1), Len::Other)) { return Err(no.into()); }
        let w = |i: usize| cols.get(i).copied().filter(|x| x.is_finite() && *x > 0.0 && cols.len() == n);
        let is_fr = |i: usize| matches!(parse_len(&items[i].1), Len::Fr(_));
        let mut set: Vec<(usize, String)> = vec![];
        match col {
            _ if vert => {
                let i = col.unwrap_or(n - 1);
                if i >= n { return Err(NO_ELEM.into()); }
                let d = match w(i) { Some(c) => dx.max(MIN_H - c), None => dx };
                set.push((i, set_len(&items[i].1, w(i), d)?));
            }
            None => {
                if (0..n).any(is_fr) { return Err(FR_EDGE.into()); }
                let d = match w(n - 1) { Some(c) => dx.max(MIN_W - c), None => dx };
                set.push((n - 1, set_len(&items[n - 1].1, w(n - 1), d)?));
            }
            Some(i) => {
                if i + 1 >= n { return Err(NO_ELEM.into()); }
                // 워드프로세서의 표처럼: 왼쪽 칸이 늘어난 만큼 오른쪽 칸이 줄어든다
                let (lf, rt) = (parse_len(&items[i].1), parse_len(&items[i + 1].1));
                // 오른쪽 칸이 함께 줄어드는 경우에만 그 칸의 남은 너비까지로 막는다
                let shrinks = matches!(rt, Len::Abs(..) | Len::Rel(..)) || matches!(lf, Len::Fr(_));
                let lo = w(i).map_or(f32::MIN, |a| (MIN_W - a).min(0.0));
                let hi = match w(i + 1) { Some(b) if shrinks => (b - MIN_W).max(0.0), _ => f32::MAX };
                let d = dx.clamp(lo, hi);
                match (lf, rt) {
                    (Len::Fr(a), Len::Fr(b)) => {
                        let (wa, wb) = (w(i).ok_or(NO_COLS)?, w(i + 1).ok_or(NO_COLS)?);
                        let na = (a + b) * (wa + d) / (wa + wb);
                        set.push((i, format!("{}fr", fmt_num(na, 2))));
                        set.push((i + 1, format!("{}fr", fmt_num(a + b - na, 2))));
                    }
                    // 채우는 칸(fr)은 저절로 맞춰지므로 그 옆 칸만 고친다
                    (Len::Fr(_), _) => set.push((i + 1, set_len(&items[i + 1].1, w(i + 1), -d)?)),
                    (_, right) => {
                        set.push((i, set_len(&items[i].1, w(i), d)?));
                        if matches!(right, Len::Abs(..) | Len::Rel(..)) { set.push((i + 1, set_len(&items[i + 1].1, w(i + 1), -d)?)); }
                    }
                }
            }
        }
        for (i, v) in &set { items[*i].1 = v.clone(); }
        if items.iter().all(|x| x.0.is_some()) {
            set.sort_by_key(|x| std::cmp::Reverse(x.0));
            for (i, v) in set { t.replace_range(items[i].0.clone().unwrap(), &v); }
        } else {
            let list: Vec<&str> = items.iter().map(|x| x.1.as_str()).collect();
            let arr = format!("({}{})", list.join(", "), if n == 1 { "," } else { "" });
            match (&val, named("columns")) {
                (Some(val), _) => t.replace_range(val.range(), &arr),
                // `rows:`가 없던 표: `columns:` 바로 뒤에 적는다
                (None, Some(c)) => t.insert_str(c.range().end, &format!(", {keyc}: {arr}")),
                (None, None) => { let (at, w) = arg_insert(&f, keyc, &arr).ok_or(NO_ELEM)?; t.insert_str(at.start, &w); }
            }
        }
    } else if SIZED.contains(&name.as_str()) {
        let d = match width { Some(c) => dx.max(if vert { MIN_H } else { MIN_W } - c), None => dx };
        match named(keyw).and_then(|n| n.children().last()) {
            Some(val) => { let v = set_len(&text[val.range()], width, d)?; t.replace_range(val.range(), &v); }
            None => {
                let v = abs_mm(width.ok_or(NO_ELEM)? + d);
                let (at, w) = arg_insert(&f, keyw, &v).ok_or(NO_ELEM)?;
                t.insert_str(at.start, &w);
            }
        }
    } else {
        return Err("이 요소는 지면에서 너비를 바꿀 수 없습니다.".into());
    }
    if bad(&tree(&t)) { return Err("이 요소는 지면에서 너비를 바꿀 수 없습니다.".into()); }
    Ok(t)
}

// ───────────────────────── 템플릿 상자: 이 곳만 바꾸기, 변수로 적힌 값

/// 원고 고침 하나: (바꿀 범위, 새 글).
pub type Edit = (Range<usize>, String);

/// 고침들을 적용한 본문.
pub fn apply_edits(text: &str, edits: &[Edit]) -> String {
    let mut v: Vec<&Edit> = edits.iter().collect();
    v.sort_by_key(|e| std::cmp::Reverse(e.0.start));
    let mut t = text.to_string();
    for (r, w) in v { t.replace_range(r.clone(), w); }
    t
}

/// 고침들을 적용한 뒤 원고 자리 `p`가 옮겨 가는 곳.
pub fn map_pos(edits: &[Edit], p: usize) -> usize {
    let mut v: Vec<&Edit> = edits.iter().collect();
    v.sort_by_key(|e| e.0.start);
    let mut shift = 0isize;
    for (r, w) in v {
        if r.is_empty() && p == r.start { break; }
        if p >= r.end { shift += w.len() as isize - r.len() as isize; }
        else if p > r.start { return (r.start as isize + shift) as usize; }
        else { break; }
    }
    (p as isize + shift) as usize
}

/// 두 본문이 다른 구간을 고침 하나로.
fn diff_edit(old: &str, new: &str) -> Vec<Edit> {
    if old == new { return vec![]; }
    let (ob, nb) = (old.as_bytes(), new.as_bytes());
    let mut pre = 0;
    while pre < ob.len() && pre < nb.len() && ob[pre] == nb[pre] { pre += 1; }
    while !old.is_char_boundary(pre) || !new.is_char_boundary(pre) { pre -= 1; }
    let mut suf = 0;
    while suf < ob.len() - pre && suf < nb.len() - pre && ob[ob.len() - 1 - suf] == nb[nb.len() - 1 - suf] { suf += 1; }
    while !old.is_char_boundary(ob.len() - suf) || !new.is_char_boundary(nb.len() - suf) { suf -= 1; }
    vec![(pre..ob.len() - suf, new[pre..nb.len() - suf].to_string())]
}

/// 부름에서 `key:`로 준 인자의 값.
fn named_arg<'a>(f: &LinkedNode<'a>, key: &str) -> Option<LinkedNode<'a>> {
    let args = f.children().find(|c| c.kind() == SyntaxKind::Args)?;
    let n = args.children().find(|c| c.kind() == SyntaxKind::Named && c.children().next().is_some_and(|k| k.get().full_text() == key))?;
    n.children().last()
}

/// 부름에 `key: value` 인자를 넣는 고침.
fn arg_insert(f: &LinkedNode, key: &str, value: &str) -> Option<Edit> {
    match f.children().find(|c| c.kind() == SyntaxKind::Args) {
        Some(args) => match args.children().next() {
            Some(lp) if lp.kind() == SyntaxKind::LeftParen => {
                let empty = args.children().nth(1).is_some_and(|c| c.kind() == SyntaxKind::RightParen);
                Some((lp.range().end..lp.range().end, if empty { format!("{key}: {value}") } else { format!("{key}: {value}, ") }))
            }
            _ => Some((args.range().start..args.range().start, format!("({key}: {value})"))),
        },
        None => None,
    }
}

/// 부름의 인자 끝에 `key: value`를 덧붙이는 고침.
fn arg_append(text: &str, f: &LinkedNode, key: &str, value: &str) -> Option<Edit> {
    let args = f.children().find(|c| c.kind() == SyntaxKind::Args)?;
    let rp = args.children().find(|c| c.kind() == SyntaxKind::RightParen);
    match (args.children().next(), rp) {
        (Some(lp), Some(rp)) if lp.kind() == SyntaxKind::LeftParen => {
            let head = text[args.range().start..rp.range().start].trim_end();
            let sep = if head.ends_with('(') { "" } else if head.ends_with(',') { " " } else { ", " };
            let at = args.range().start + head.len();
            Some((at..at, format!("{sep}{key}: {value}")))
        }
        _ => Some((args.range().start..args.range().start, format!("({key}: {value})"))),
    }
}

/// 상자·표의 크기를 정하는 인자의 이름.
fn size_key(name: &str, vert: bool) -> Option<&'static str> {
    if name == "grid" || name == "table" { Some(if vert { "rows" } else { "columns" }) }
    else if SIZED.contains(&name) { Some(if vert { "height" } else { "width" }) } else { None }
}

/// 값 표기 하나를 끈 만큼 고친 새 표기: 같은 규칙을 쓰려고 그 값만 가진 부름을 만들어 고쳐 본다.
fn recompute(name: &str, key: &str, old: Option<&str>, col: Option<usize>, dx: f32, width: Option<f32>, cols: &[f32]) -> R<String> {
    let tmp = match old { Some(v) => format!("#{name}({key}: {v})"), None => format!("#{name}()") };
    let out = resize_ax(&tmp, &(1..tmp.len()), key == "rows" || key == "height", col, dx, width, cols)?;
    let node = tree(&out);
    let root = LinkedNode::new(&node);
    let f = root.leaf_at(2, Side::Before).and_then(|l| func_of(&l)).ok_or(NO_ELEM)?;
    named_arg(&f, key).map(|v| out[v.range()].to_string()).ok_or_else(|| NO_ELEM.to_string())
}

/// `f`를 품은 함수 정의(`#let 이름(..) = ..`): (정의, 이름).
fn closure_of<'a>(f: &LinkedNode<'a>) -> Option<(LinkedNode<'a>, String)> {
    let mut n = f.parent()?.clone();
    loop {
        if n.kind() == SyntaxKind::Closure {
            let name = n.children().next().filter(|c| c.kind() == SyntaxKind::Ident).map(|c| c.get().full_text().to_string())?;
            return Some((n, name));
        }
        n = n.parent()?.clone();
    }
}

/// 함수 정의에서 이름 붙은 매개변수 `p`의 기본값 범위.
fn param_default(clo: &LinkedNode, p: &str) -> Option<Range<usize>> {
    let params = clo.children().find(|c| c.kind() == SyntaxKind::Params)?;
    let n = params.children().find(|c| c.kind() == SyntaxKind::Named && c.children().next().is_some_and(|k| k.get().full_text() == p))?;
    n.children().last().map(|v| v.range())
}

/// 함수 정의에 `p`라는 매개변수(자리로 받는 것 포함)가 있는지.
fn has_param(clo: &LinkedNode, p: &str) -> bool {
    clo.children().filter(|c| c.kind() == SyntaxKind::Params).any(|ps| ps.children().any(|c| match c.kind() {
        SyntaxKind::Ident => c.get().full_text() == p,
        SyntaxKind::Named | SyntaxKind::Spread => c.children().any(|k| k.kind() == SyntaxKind::Ident && k.get().full_text() == p),
        _ => false,
    }))
}

/// `#let p = 값`으로 정한 변수의 값 범위(`before` 앞에서 가장 가까운 것).
fn let_value(root: &LinkedNode, p: &str, before: usize) -> Option<Range<usize>> {
    fn go(n: &LinkedNode, p: &str, before: usize, best: &mut Option<Range<usize>>) {
        if n.range().start >= before { return; }
        if n.kind() == SyntaxKind::LetBinding {
            let ch: Vec<_> = n.children().collect();
            let name_ok = ch.iter().find(|c| !matches!(c.kind(), SyntaxKind::Let | SyntaxKind::Space)).is_some_and(|c| c.kind() == SyntaxKind::Ident && c.get().full_text() == p);
            if name_ok && ch.iter().any(|c| c.kind() == SyntaxKind::Eq) {
                if let Some(v) = ch.last() { if v.kind() != SyntaxKind::Closure && v.range().end <= before { *best = Some(v.range()); } }
            }
        }
        for c in n.children() { go(&c, p, before, best); }
    }
    let mut best = None;
    go(root, p, before, &mut best);
    best
}

/// 상자·표를 끌어 바꿀 때의 고침: 상자가 적힌 원고(`def`)와, 그 상자를 부른 자리의 원고(`site`).
pub struct BoxEdit { pub def: Vec<Edit>, pub site: Vec<Edit>, /// 이 곳만 바꿨는지(부른 자리에 값을 줬는지)
    pub local: bool }

const NO_VAR: &str = "이 너비는 원고에 수로 적혀 있지 않아(식이나 다른 곳에서 온 값) 지면에서 바꿀 수 없습니다.";

/// 상자가 템플릿 함수 안에 있고 그 함수를 부른 자리를 알면: (함수 정의, 부른 자리의 부름 범위).
fn local_target<'a>(f: &LinkedNode<'a>, site: Option<(&str, usize)>) -> Option<(LinkedNode<'a>, Range<usize>)> {
    let (clo, fname) = closure_of(f)?;
    let (st, pos) = site?;
    let call = elem_chain(st, pos).into_iter().find(|e| e.name == fname)?;
    Some((clo, call.range))
}

/// 이 상자만 따로 바꿀 수 있는지(템플릿 함수가 그린 상자이고, 그 함수를 부른 자리를 안다).
pub fn box_can_local(def: &str, r: &Range<usize>, site: Option<(&str, usize)>) -> bool {
    let node = tree(def);
    let root = LinkedNode::new(&node);
    find_node(&root, r, SyntaxKind::FuncCall).is_some_and(|f| local_target(&f, site).is_some())
}

/// 상자·표의 너비를 `dx`pt 만큼 바꾸는 고침을 만든다.
/// - 값이 변수면 그 변수가 정해진 곳(함수의 매개변수 기본값, `#let 변수 = 값`)을 고친다.
/// - `local`: 템플릿 함수가 그린 상자면 이 곳만 바꾼다. 함수에 매개변수(`width: 100%`)를 만들고,
///   부른 자리에 값(`#key(width: 80%)[..]`)을 준다. 부른 자리(`site`: 본문과 자리)를 모르면 모두 바꾼다.
/// `vert`: 높이를 바꾼다(`col`은 줄 번호, `width`는 전체 높이, `cols`는 줄마다의 높이).
pub fn box_resize(def: &str, r: &Range<usize>, site: Option<(&str, usize)>, local: bool, vert: bool,
                  col: Option<usize>, dx: f32, width: Option<f32>, cols: &[f32]) -> R<BoxEdit> {
    let node = tree(def);
    let root = LinkedNode::new(&node);
    let f = find_node(&root, r, SyntaxKind::FuncCall).ok_or(NO_ELEM)?;
    let name = callee_name(&f);
    let key = size_key(&name, vert).ok_or("이 요소는 지면에서 너비를 바꿀 수 없습니다.")?;
    let val = named_arg(&f, key);
    let clo = closure_of(&f).map(|c| c.0);
    // 값이 변수면: 어디에서 정해졌는가
    enum Src { Direct, Param(String, Range<usize>), Var(Range<usize>) }
    let src = match &val {
        Some(v) if v.kind() == SyntaxKind::Ident => {
            let p = v.get().full_text().to_string();
            match clo.as_ref().and_then(|c| param_default(c, &p)) {
                Some(d) => Src::Param(p, d),
                None => match let_value(&root, &p, f.range().start) {
                    Some(d) => Src::Var(d),
                    None => return Err(match key { "columns" => NO_COLS, "rows" => NO_ROWS, _ => NO_VAR }.into()),
                },
            }
        }
        _ => Src::Direct,
    };
    if local {
        if let Some((clo, call)) = local_target(&f, site) {
            let (st, _) = site.unwrap();
            let snode = tree(st);
            let sroot = LinkedNode::new(&snode);
            let cf = find_node(&sroot, &call, SyntaxKind::FuncCall).ok_or(NO_ELEM)?;
            // 매개변수 이름과, 이 곳에 지금 적용되는 값
            let (pname, dflt): (String, Option<String>) = match &src {
                Src::Param(p, d) => (p.clone(), Some(def[d.clone()].to_string())),
                Src::Var(d) => (key.to_string(), Some(def[d.clone()].to_string())),
                Src::Direct => (key.to_string(), val.as_ref().map(|v| def[v.range()].to_string())),
            };
            let fresh = !matches!(src, Src::Param(..));
            if !(fresh && has_param(&clo, &pname)) {
                let cur = named_arg(&cf, &pname);
                let old = cur.as_ref().map(|v| st[v.range()].to_string()).or(dflt);
                let new = recompute(&name, key, old.as_deref(), col, dx, width, cols)?;
                let site_edit = match cur {
                    Some(v) => (v.range(), new),
                    None => arg_append(st, &cf, &pname, &new).ok_or(NO_ELEM)?,
                };
                let mut de: Vec<Edit> = vec![];
                if fresh {
                    // 함수에 매개변수를 만든다: 기본값은 지금 적힌 값 그대로
                    let params = clo.children().find(|c| c.kind() == SyntaxKind::Params).ok_or(NO_ELEM)?;
                    let rp = params.children().last().filter(|c| c.kind() == SyntaxKind::RightParen).ok_or(NO_ELEM)?;
                    let head = def[params.range().start..rp.range().start].trim_end();
                    let sep = if head.ends_with('(') || head.ends_with(',') { if head.ends_with(',') { " " } else { "" } } else { ", " };
                    let at = params.range().start + head.len();
                    let dv = val.as_ref().map(|v| def[v.range()].to_string()).unwrap_or_else(|| "auto".into());
                    de.push((at..at, format!("{sep}{pname}: {dv}")));
                    match &val {
                        Some(v) => de.push((v.range(), pname.clone())),
                        None => de.push(arg_insert(&f, key, &pname).ok_or(NO_ELEM)?),
                    }
                }
                return Ok(BoxEdit { def: de, site: vec![site_edit], local: true });
            }
        }
    }
    // 모두 바꾸기: 값이 적힌 곳을 고친다
    let de = match src {
        Src::Direct => diff_edit(def, &resize_ax(def, r, vert, col, dx, width, cols)?),
        Src::Param(_, d) | Src::Var(d) => vec![(d.clone(), recompute(&name, key, Some(&def[d]), col, dx, width, cols)?)],
    };
    Ok(BoxEdit { def: de, site: vec![], local: false })
}

// ───────────────────────── 그림

/// 부름에 `key:` 인자가 있는지.
pub fn has_named(text: &str, r: &Range<usize>, key: &str) -> bool {
    let node = tree(text);
    let root = LinkedNode::new(&node);
    find_node(&root, r, SyntaxKind::FuncCall).is_some_and(|f| named_arg(&f, key).is_some())
}

/// 템플릿 함수 `name`의 본문이 곧 상자 하나(`#let name(..) = block(..)[..]`)면 그 부름의 범위.
/// 채움도 테두리도 없는 상자는 지면에 출처가 남지 않아, 함수의 생김새로 미루어 안다.
pub fn fn_sized_body(text: &str, name: &str) -> Option<Range<usize>> {
    fn go(n: &LinkedNode, name: &str) -> Option<Range<usize>> {
        if n.kind() == SyntaxKind::Closure {
            let ch: Vec<_> = n.children().collect();
            if ch.first().is_some_and(|c| c.kind() == SyntaxKind::Ident && c.get().full_text() == name) {
                let body = ch.last()?;
                // 표·격자는 지면에 출처가 남으므로 여기서는 상자만 본다
                return (body.kind() == SyntaxKind::FuncCall && SIZED.contains(&callee_name(body).as_str())).then(|| body.range());
            }
        }
        n.children().find_map(|c| go(&c, name))
    }
    let node = tree(text);
    go(&LinkedNode::new(&node), name)
}

/// 본문에서 `name`을 부른 곳의 수.
pub fn count_calls(text: &str, name: &str) -> usize {
    fn go(n: &LinkedNode, name: &str) -> usize {
        (n.kind() == SyntaxKind::FuncCall && callee_name(n) == name) as usize + n.children().map(|c| go(&c, name)).sum::<usize>()
    }
    let node = tree(text);
    go(&LinkedNode::new(&node), name)
}

/// 이 부름이 템플릿 함수(`#let 이름(..) = ..`) 안에 있으면 그 함수의 이름.
pub fn closure_name(text: &str, r: &Range<usize>) -> Option<String> {
    let node = tree(text);
    let root = LinkedNode::new(&node);
    let f = find_node(&root, r, SyntaxKind::FuncCall)?;
    closure_of(&f).map(|c| c.1)
}

/// 부름의 자리 인자들(쉼표로 준 것과 뒤에 붙인 글 상자), 차례대로.
fn positional<'a>(f: &LinkedNode<'a>) -> Vec<LinkedNode<'a>> {
    let Some(args) = f.children().find(|c| c.kind() == SyntaxKind::Args) else { return vec![] };
    args.children().filter(|c| !matches!(c.kind(), SyntaxKind::LeftParen | SyntaxKind::RightParen | SyntaxKind::Comma | SyntaxKind::Space
        | SyntaxKind::Named | SyntaxKind::Spread | SyntaxKind::LineComment | SyntaxKind::BlockComment)).collect()
}

/// 그림의 파일 경로가 글자로 적힌 곳: (부른 자리의 원고에 있는지, 따옴표를 포함한 범위).
/// `image("a.png")`면 그 자리, `image(path)`처럼 변수면 템플릿 함수를 부른 자리에서 그 값을 준 곳.
fn image_path_at(def: &str, r: &Range<usize>, site: Option<(&str, usize)>) -> Option<(bool, Range<usize>)> {
    let node = tree(def);
    let root = LinkedNode::new(&node);
    let f = find_node(&root, r, SyntaxKind::FuncCall)?;
    if callee_name(&f) != "image" { return None; }
    let first = positional(&f).into_iter().next()?;
    match first.kind() {
        SyntaxKind::Str => Some((false, first.range())),
        SyntaxKind::Ident => {
            let p = first.get().full_text().to_string();
            let (clo, fname) = closure_of(&f)?;
            let (st, pos) = site?;
            let call = elem_chain(st, pos).into_iter().find(|e| e.name == fname)?;
            let snode = tree(st);
            let sroot = LinkedNode::new(&snode);
            let cf = find_node(&sroot, &call.range, SyntaxKind::FuncCall)?;
            // 이름 붙은 매개변수면 부른 자리의 그 인자, 자리 매개변수면 같은 차례의 인자
            if let Some(v) = named_arg(&cf, &p) { return (v.kind() == SyntaxKind::Str).then(|| (true, v.range())); }
            let params = clo.children().find(|c| c.kind() == SyntaxKind::Params)?;
            let k = params.children().filter(|c| c.kind() == SyntaxKind::Ident).position(|c| c.get().full_text() == p.as_str());
            match k {
                Some(k) => positional(&cf).into_iter().nth(k).filter(|v| v.kind() == SyntaxKind::Str).map(|v| (true, v.range())),
                None => param_default(&clo, &p).filter(|d| def[d.clone()].starts_with('"')).map(|d| (false, d)),
            }
        }
        _ => None,
    }
}

/// 그림의 파일 경로(원고에 적힌 그대로, 따옴표 없이).
pub fn image_path(def: &str, r: &Range<usize>, site: Option<(&str, usize)>) -> Option<String> {
    let (at_site, range) = image_path_at(def, r, site)?;
    let t = if at_site { site?.0 } else { def };
    Some(t[range].trim_matches('"').to_string())
}

/// 그림의 파일 경로를 `new`로 바꾸는 고침.
pub fn image_replace(def: &str, r: &Range<usize>, site: Option<(&str, usize)>, new: &str) -> R<BoxEdit> {
    let (at_site, range) = image_path_at(def, r, site).ok_or("이 그림의 파일 이름은 원고에 글자로 적혀 있지 않아 여기서 바꿀 수 없습니다.")?;
    let lit = format!("\"{}\"", new.replace('\\', "/").replace('"', "\\\""));
    Ok(if at_site { BoxEdit { def: vec![], site: vec![(range, lit)], local: true } } else { BoxEdit { def: vec![(range, lit)], site: vec![], local: false } })
}

// ───────────────────────── 표의 줄·칸

const NO_TABLE: &str = "이 표는 원고에 칸이 하나씩 적혀 있지 않아(코드로 만든 표, 합친 칸, 머리줄 묶음) 줄·칸을 넣거나 지울 수 없습니다.";

/// 칸이 원고에 하나씩 적힌 표·격자.
struct Tbl {
    call: Range<usize>,
    ncols: usize,
    /// 칸마다의 원고 범위(왼쪽 위부터 줄 차례로)
    items: Vec<Range<usize>>,
    /// `columns:`가 배열이면 그 항목들, 수면 그 수의 범위
    cols_arr: Option<Vec<Range<usize>>>,
    cols_int: Option<Range<usize>>,
    /// `rows:`가 배열이면 그 항목들
    rows_arr: Option<Vec<Range<usize>>>,
    /// 여는 괄호 바로 뒤(인자를 새로 넣을 자리)
    lp: usize,
}

/// 쉼표로 늘어놓은 배열의 항목 범위들. 펼침(`..`)이나 이름 붙은 항목이 있으면 None.
fn list_ranges(arr: &LinkedNode) -> Option<Vec<Range<usize>>> {
    let mut v = vec![];
    for c in arr.children() {
        match c.kind() {
            SyntaxKind::LeftParen | SyntaxKind::RightParen | SyntaxKind::Comma | SyntaxKind::Space | SyntaxKind::LineComment | SyntaxKind::BlockComment => {}
            SyntaxKind::Spread | SyntaxKind::Named | SyntaxKind::Keyed => return None,
            _ => v.push(c.range()),
        }
    }
    Some(v)
}

fn tbl(f: &LinkedNode, text: &str) -> Option<Tbl> {
    let name = callee_name(f);
    if name != "table" && name != "grid" { return None; }
    let args = f.children().find(|c| c.kind() == SyntaxKind::Args)?;
    let lp = args.children().next().filter(|c| c.kind() == SyntaxKind::LeftParen)?.range().end;
    let mut items = vec![];
    let mut closed = false;
    for c in args.children() {
        match c.kind() {
            SyntaxKind::LeftParen | SyntaxKind::Comma | SyntaxKind::Space | SyntaxKind::LineComment | SyntaxKind::BlockComment | SyntaxKind::Named => {}
            SyntaxKind::RightParen => closed = true,
            SyntaxKind::Spread => return None,
            // 합친 칸·머리줄 묶음·괘선(`table.cell(..)`, `table.header(..)`, `table.hline()` …)이 있으면 칸을 셀 수 없다
            SyntaxKind::FieldAccess => return None,
            SyntaxKind::FuncCall if callee_name(&c).contains('.') => return None,
            // 괄호 뒤에 붙여 쓴 글 상자
            _ if closed => return None,
            _ => items.push(c.range()),
        }
    }
    let (cols_arr, cols_int, ncols) = match named_arg(f, "columns") {
        None => (None, None, 1),
        Some(v) if v.kind() == SyntaxKind::Array => { let l = list_ranges(&v)?; let n = l.len(); (Some(l), None, n) }
        Some(v) if v.kind() == SyntaxKind::Int => { let n: usize = text[v.range()].trim().parse().ok()?; (None, Some(v.range()), n) }
        Some(_) => return None,
    };
    if ncols == 0 || items.is_empty() { return None; }
    let rows_arr = named_arg(f, "rows").filter(|v| v.kind() == SyntaxKind::Array).and_then(|v| list_ranges(&v));
    Some(Tbl { call: f.range(), ncols, items, cols_arr, cols_int, rows_arr, lp })
}

/// `pos`가 든 칸을 가진 가장 안쪽 표: (표, 칸 번호).
fn table_find(root: &LinkedNode, text: &str, pos: usize) -> Option<(Tbl, usize)> {
    for side in [Side::Before, Side::After] {
        let Some(leaf) = root.leaf_at(pos, side) else { continue };
        let mut n = leaf;
        loop {
            if n.kind() == SyntaxKind::Closure { break; }
            if n.kind() == SyntaxKind::FuncCall {
                if let Some(t) = tbl(&n, text) {
                    if let Some(i) = t.items.iter().position(|r| r.start <= pos && pos <= r.end) { return Some((t, i)); }
                }
            }
            match n.parent() { Some(p) => n = p.clone(), None => break }
        }
    }
    None
}

fn node_at<'a>(node: &LinkedNode<'a>, start: usize, kind: SyntaxKind) -> Option<LinkedNode<'a>> {
    if node.range().start == start && node.kind() == kind { return Some(node.clone()); }
    for c in node.children() {
        let r = c.range();
        if r.start <= start && start < r.end.max(r.start + 1) { if let Some(f) = node_at(&c, start, kind) { return Some(f); } }
    }
    None
}

fn cell_blank(text: &str, r: &Range<usize>) -> bool {
    let t = &text[r.clone()];
    t.starts_with('[') && t.ends_with(']') && t[1..t.len() - 1].trim().is_empty()
}
/// 칸 안에서 커서를 둘 자리.
fn cell_caret(text: &str, r: &Range<usize>) -> usize { if text[r.clone()].starts_with('[') { r.start + 1 } else { r.start } }

/// 커서가 든 표의 칸.
pub struct TableAt { pub call: Range<usize>, pub col: usize, pub row: usize, pub ncols: usize, pub nrows: usize, pub empty: bool }

pub fn table_where(text: &str, pos: usize) -> Option<TableAt> {
    let node = tree(text);
    let root = LinkedNode::new(&node);
    let (t, i) = table_find(&root, text, pos)?;
    Some(TableAt { col: i % t.ncols, row: i / t.ncols, ncols: t.ncols, nrows: t.items.len().div_ceil(t.ncols), empty: cell_blank(text, &t.items[i]), call: t.call })
}

/// 표 `call`의 (col, row) 칸: (그 칸에 커서를 둘 자리, 칸이 비었는지). 글 상자(`[..]`)로 적힌 칸만.
pub fn table_cell(text: &str, call: &Range<usize>, col: usize, row: usize) -> Option<(usize, bool)> {
    let node = tree(text);
    let root = LinkedNode::new(&node);
    let f = find_node(&root, call, SyntaxKind::FuncCall)?;
    let t = tbl(&f, text)?;
    if col >= t.ncols { return None; }
    let r = t.items.get(row * t.ncols + col)?;
    text[r.clone()].starts_with('[').then(|| (r.start + 1, cell_blank(text, r)))
}

/// 쉼표 목록에서 항목 하나를 지울 범위: 뒤의 쉼표와 공백까지, 마지막 항목이면 앞의 쉼표부터.
fn list_del(text: &str, r: &Range<usize>) -> Range<usize> {
    let rest = text[r.end..].trim_start_matches([' ', '\t']);
    if rest.starts_with(',') {
        let mut e = text.len() - rest.len() + 1;
        let after = &text[e..];
        if after.trim_start_matches([' ', '\t']).starts_with(['\n', '\r']) || after.trim_start_matches([' ', '\t']).is_empty() {
            // 줄의 마지막 항목: 앞의 공백을 지워 줄 끝에 빈칸이 남지 않게 한다
            let s = text[..r.start].trim_end_matches([' ', '\t']).len();
            if !text[..s].ends_with(['\n', '\r']) && !text[..s].ends_with('(') { return s..e; }
            return r.start..e;
        }
        e += after.len() - after.trim_start_matches(' ').len();
        r.start..e
    } else {
        let head = text[..r.start].trim_end();
        if head.ends_with(',') { head.len() - 1..r.end } else { r.clone() }
    }
}

/// 배열에서 `i`번째 항목을 지우는 고침. 항목이 하나만 남으면 `(값,)`으로 적어 배열로 남긴다.
fn arr_del(text: &str, items: &[Range<usize>], i: usize) -> Edit {
    if items.len() == 2 {
        let keep = &text[items[1 - i].clone()];
        let comma = text[items[1].end..].trim_start().starts_with(',');
        return (items[0].start..items[1].end, if comma { keep.to_string() } else { format!("{keep},") });
    }
    (list_del(text, &items[i]), String::new())
}

/// 표의 줄·칸을 넣거나 지운다. `pos`: 커서(그 칸이 기준이다).
/// `kind`: rowAbove · rowBelow · colLeft · colRight · rowDel · colDel. 돌려주는 것: (새 본문, 새 커서 자리).
pub fn table_op(text: &str, pos: usize, kind: &str) -> R<(String, usize)> {
    let node = tree(text);
    let root = LinkedNode::new(&node);
    let (t, idx) = table_find(&root, text, pos).ok_or(NO_TABLE)?;
    let (n, nc) = (t.items.len(), t.ncols);
    let (row, col) = (idx / nc, idx % nc);
    let nrows = n.div_ceil(nc);
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let cells = |k: usize| vec!["[]"; k].join(", ");
    let line_start = |p: usize| text[..p].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_first = |p: usize| text[line_start(p)..p].trim().is_empty();
    // 항목 뒤에 쉼표가 있으면 그 쉼표의 끝
    let comma_after = |e: usize| { let rest = text[e..].trim_start_matches([' ', '\t']); rest.starts_with(',').then(|| text.len() - rest.len() + 1) };
    let mut ed: Vec<Edit> = vec![];
    let (mut tr, mut tc) = (row, col);
    match kind {
        "rowAbove" | "rowBelow" => {
            let at = if kind == "rowAbove" { row } else { row + 1 };
            if at < nrows {
                let first = t.items[at * nc].start;
                if line_first(first) { ed.push((first..first, format!("{},{eol}{}", cells(nc), &text[line_start(first)..first]))); }
                else { ed.push((first..first, format!("{}, ", cells(nc)))); }
            } else {
                // 맨 끝에 덧붙인다(마지막 줄이 덜 찼으면 먼저 채운다)
                let k = (nc - n % nc) % nc + nc;
                let last = &t.items[n - 1];
                let head = t.items[(nrows - 1) * nc].start;
                let fill = (nc - n % nc) % nc;
                let body = if fill == 0 { cells(nc) } else { cells(k) };
                match (comma_after(last.end), line_first(head) && fill == 0) {
                    (Some(c), true) => ed.push((c..c, format!("{eol}{}{body},", &text[line_start(head)..head]))),
                    (None, true) => ed.push((last.end..last.end, format!(",{eol}{}{body}", &text[line_start(head)..head]))),
                    (Some(c), false) => ed.push((c..c, format!(" {body},"))),
                    (None, false) => ed.push((last.end..last.end, format!(", {body}"))),
                }
            }
            if let Some(rs) = t.rows_arr.as_ref().filter(|rs| !rs.is_empty()) {
                if at < rs.len() { ed.push((rs[at].start..rs[at].start, "auto, ".into())); }
                else if rs.len() == nrows { let e = rs[rs.len() - 1].end; ed.push((e..e, ", auto".into())); }
            }
            tr = at;
        }
        "rowDel" => {
            if nrows <= 1 { return Err("마지막 줄은 지울 수 없습니다. 표를 통째로 지우려면 Alt+누르기로 골라 지워 주세요.".into()); }
            let (a, b) = (row * nc, (row * nc + nc).min(n));
            let (first, last_end) = (t.items[a].start, t.items[b - 1].end);
            match comma_after(last_end) {
                Some(e) => {
                    let le = text[e..].find('\n').map(|i| e + i + 1).unwrap_or(text.len());
                    if line_first(first) && text[e..le].trim().is_empty() { ed.push((line_start(first)..le, String::new())); }
                    else { let e2 = e + (text[e..].len() - text[e..].trim_start_matches(' ').len()); ed.push((first..e2, String::new())); }
                }
                None => {
                    let head = text[..first].trim_end();
                    ed.push((if head.ends_with(',') { head.len() - 1 } else { first }..last_end, String::new()));
                }
            }
            if let Some(rs) = t.rows_arr.as_ref().filter(|rs| rs.len() > 1) {
                if row < rs.len() { ed.push(arr_del(text, rs, row)); }
            }
            tr = row.saturating_sub(1);
        }
        "colLeft" | "colRight" => {
            let at = if kind == "colLeft" { col } else { col + 1 };
            for r in 0..nrows {
                let k = r * nc + at;
                if at < nc && k < n { let p = t.items[k].start; ed.push((p..p, "[], ".into())); }
                else if k >= 1 && k <= n && (at == nc || k == n) { let e = t.items[k - 1].end; ed.push((e..e, ", []".into())); }
            }
            match (&t.cols_arr, &t.cols_int) {
                (Some(cs), _) => {
                    let spec = text[cs[col.min(cs.len() - 1)].clone()].to_string();
                    if at < cs.len() { ed.push((cs[at].start..cs[at].start, format!("{spec}, "))); }
                    else { let e = cs[cs.len() - 1].end; ed.push((e..e, format!(", {spec}"))); }
                }
                (None, Some(r)) => ed.push((r.clone(), (nc + 1).to_string())),
                (None, None) => ed.push((t.lp..t.lp, "columns: 2, ".into())),
            }
            tc = at;
        }
        "colDel" => {
            if nc <= 1 { return Err("마지막 칸은 지울 수 없습니다. 표를 통째로 지우려면 Alt+누르기로 골라 지워 주세요.".into()); }
            for r in 0..nrows {
                let k = r * nc + col;
                if k < n { ed.push((list_del(text, &t.items[k]), String::new())); }
            }
            match (&t.cols_arr, &t.cols_int) {
                (Some(cs), _) => ed.push(arr_del(text, cs, col)),
                (None, Some(r)) => ed.push((r.clone(), (nc - 1).to_string())),
                (None, None) => {}
            }
            tc = col.min(nc - 2);
        }
        _ => return Err(NO_TABLE.into()),
    }
    let out = apply_edits(text, &ed);
    let node2 = tree(&out);
    if bad(&node2) { return Err(NO_TABLE.into()); }
    // 새 표에서 커서를 둘 칸
    let root2 = LinkedNode::new(&node2);
    let caret = node_at(&root2, t.call.start, SyntaxKind::FuncCall).and_then(|f| tbl(&f, &out)).and_then(|t2| {
        let k = (tr * t2.ncols + tc).min(t2.items.len().saturating_sub(1));
        t2.items.get(k).map(|r| cell_caret(&out, r))
    }).unwrap_or(t.call.start);
    Ok((out, caret))
}

// ───────────────────────── 수식

/// 수식 원문(`$..$`)을 안쪽 글과 블록 여부로 나눈다. 블록 수식은 `$` 바로 안쪽이 공백이다.
pub fn math_parts(src: &str) -> (String, bool) {
    let inner = src.strip_prefix('$').and_then(|x| x.strip_suffix('$')).unwrap_or(src);
    let block = inner.len() >= 2 && inner.starts_with(char::is_whitespace) && inner.ends_with(char::is_whitespace);
    (inner.trim().to_string(), block)
}

/// 안쪽 글과 블록 여부로 수식 원문을 만든다. 옛 원문이 있으면 그 바깥 공백(줄바꿈·들여쓰기)을 살린다.
pub fn math_build(old: Option<&str>, body: &str, block: bool) -> String {
    let body = body.trim();
    if !block { return format!("${body}$"); }
    if let Some(o) = old {
        let inner = o.strip_prefix('$').and_then(|x| x.strip_suffix('$')).unwrap_or(o);
        let (_, was_block) = math_parts(o);
        if was_block {
            let lead = &inner[..inner.len() - inner.trim_start().len()];
            let trail = &inner[inner.trim_end().len()..];
            if !body.contains('\n') || lead.contains('\n') { return format!("${lead}{body}{trail}$"); }
        }
    }
    if body.is_empty() { "$ $".into() } else { format!("$ {body} $") }
}

/// 수식 노드가 이 구간에 있는지.
fn equation_at(text: &str, r: &Range<usize>) -> bool {
    let node = tree(text);
    find_node(&LinkedNode::new(&node), r, SyntaxKind::Equation).is_some()
}

/// `r`에 있는 수식을 `new`(원문 `$..$`, 빈 글이면 수식을 지움)로 바꾼 본문.
pub fn math_replace(text: &str, r: &Range<usize>, new: &str) -> R<String> {
    if !equation_at(text, r) { return Err("고칠 수식을 찾지 못했습니다.".into()); }
    let mut t = text.to_string();
    if new.is_empty() {
        // 수식을 지우면 그 앞뒤 공백이 겹치지 않게 한쪽을 함께 지운다
        let mut rr = r.clone();
        let before = t[..rr.start].chars().next_back();
        let after = t[rr.end..].chars().next();
        if before == Some(' ') && matches!(after, Some(' ') | Some('\n') | Some('\r') | None) { rr.start -= 1; }
        else if after == Some(' ') && matches!(before, Some('\n') | None) { rr.end += 1; }
        t.replace_range(rr, "");
        if bad(&tree(&t)) { return Err("수식을 지우면 조판 오류가 나서 지우지 않았습니다.".into()); }
        return Ok(t);
    }
    t.replace_range(r.clone(), new);
    let nr = r.start..r.start + new.len();
    if bad(&tree(&t)) || !equation_at(&t, &nr) { return Err("수식 문법이 맞지 않습니다.".into()); }
    Ok(t)
}

/// 커서 자리에 수식 원문을 넣는다. 넣은 수식의 구간을 돌려준다.
pub fn math_insert(w: &mut Work, src: &str) -> R<Range<usize>> { math_insert_opt(w, src, true) }

/// `space`: 낱말에 바로 붙으면 사이를 띄울지(붙여 넣는 글처럼 띄어쓰기를 이미 갖춘 경우는 false).
pub fn math_insert_opt(w: &mut Work, src: &str, space: bool) -> R<Range<usize>> {
    if w.sel().is_some() { delete_sel_opt(w, false)?; }
    let p = w.caret;
    let node = tree(&w.text);
    let root = LinkedNode::new(&node);
    if !matches!(ctx(&root, p), Ctx::Markup) { return Err("이 자리에는 수식을 넣을 수 없습니다. 본문 글 사이를 눌러 커서를 놓아 주세요.".into()); }
    let guard = if expr_ends_at(&root, p).is_some() { ";" } else { "" };
    // 낱말에 바로 붙여 넣으면 사이를 띄운다(글줄 안 수식이 낱말에 붙지 않게)
    let before = if space && guard.is_empty() && prev_char(&w.text, p).is_some_and(char::is_alphanumeric) { " " } else { "" };
    let after = if space && next_char(&w.text, p).is_some_and(char::is_alphanumeric) { " " } else { "" };
    let lead = format!("{guard}{before}");
    let ins = format!("{lead}{src}{after}");
    let mut t = w.text.clone();
    t.insert_str(p, &ins);
    let r = p + lead.len()..p + lead.len() + src.len();
    if bad(&tree(&t)) || !equation_at(&t, &r) { return Err("이 자리에는 수식을 넣을 수 없습니다.".into()); }
    w.text = t;
    let end = r.end + after.len();
    w.caret = end; w.anchor = end;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(text: &str) -> Work {
        // `|` 커서, `<`..`>` 대신 `{`,`}` 로 선택
        if let (Some(a), Some(b)) = (text.find('{'), text.find('}')) {
            let t = text.replace(['{', '}'], "");
            Work::new(t, b - 1, a)
        } else {
            let p = text.find('|').unwrap();
            Work::new(text.replace('|', ""), p, p)
        }
    }
    fn show(w: &Work) -> String { let mut t = w.text.clone(); t.insert(w.caret, '|'); t }
    #[test] fn math_parts_build() {
        assert_eq!(math_parts("$x^2$"), ("x^2".into(), false));
        assert_eq!(math_parts("$ a + b $"), ("a + b".into(), true));
        assert_eq!(math_build(Some("$\n  a\n$"), "b + c", true), "$\n  b + c\n$");
        assert_eq!(math_build(Some("$a$"), "b", true), "$ b $");
        assert_eq!(math_build(Some("$ a $"), "b", false), "$b$");
    }
    #[test] fn math_rep() {
        let t = "Area $pi r^2$ here.";
        let r = 5..13;
        assert_eq!(math_replace(t, &r, "$pi r^3$").unwrap(), "Area $pi r^3$ here.");
        assert!(math_replace(t, &r, "$pi \"r$").is_err());
        assert!(math_replace(t, &r, "$a$ $b$").is_err());
        assert_eq!(math_replace(t, &r, "").unwrap(), "Area here.");
    }
    #[test] fn math_ins() {
        let mut x = w("Let |be."); let r = math_insert(&mut x, "$x$").unwrap();
        assert_eq!(&x.text[r], "$x$"); assert_eq!(x.text, "Let $x$ be.");
        let mut z = w("a|b"); let r = math_insert(&mut z, "$x$").unwrap(); assert_eq!(z.text, "a $x$ b"); assert_eq!(&z.text[r], "$x$");
        let mut y = w("#f(|)"); assert!(math_insert(&mut y, "$x$").is_err());
    }


    #[test] fn el_chain() { let t = "#let f() = {\n  place(top, dx: 1mm, block({\n    text(size: 1pt)[가나]\n  }))\n}"; let c = elem_chain(t, t.find("가나").unwrap()); assert_eq!(c.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["text", "block", "place"]); }
    #[test] fn el_del_stmt() { let t = "#let f() = {\n  v(1mm)\n  text[가]\n  v(2mm)\n}"; let c = elem_chain(t, t.find("가").unwrap()); assert_eq!(elem_delete(t, &c[0].range).unwrap(), "#let f() = {\n  v(1mm)\n  v(2mm)\n}"); }
    #[test] fn el_del_markup() { let t = "가 #pos(\"명\") 나"; let c = elem_chain(t, t.find("pos").unwrap()); assert_eq!(elem_delete(t, &c[0].range).unwrap(), "가  나"); }
    #[test] fn el_del_arg() { let t = "#place(top, text[가])"; let c = elem_chain(t, t.find("가").unwrap()); assert_eq!(elem_delete(t, &c[0].range).unwrap(), "#place(top, [])"); }
    #[test] fn el_move_place() { let t = "#place(top, dx: 24mm, dy: -24mm, [가])"; let c = elem_chain(t, t.find("가").unwrap()); let (n, r) = elem_move(t, &c[0].range, 0.0, 2.834_645_7).unwrap(); assert_eq!(n, "#place(top, dx: 24mm, dy: -23mm, [가])"); assert_eq!(&n[r], "place(top, dx: 24mm, dy: -23mm, [가])"); }
    #[test] fn el_move_wrap() { let t = "#let f() = {\n  v(1mm)\n  text(size: 1pt)[가]\n}"; let c = elem_chain(t, t.find("가").unwrap()); let (n, r) = elem_move(t, &c[0].range, 0.0, -3.0).unwrap(); assert_eq!(n, "#let f() = {\n  v(1mm)\n  { let it = text(size: 1pt)[가]; place(dx: 0pt, dy: -3pt, it); hide(it) }\n}"); let (n2, _) = elem_move(&n, &r, 1.5, 1.0).unwrap(); assert!(n2.contains("place(dx: 1.5pt, dy: -2pt, it)")); let c2 = elem_chain(&n2, n2.find("가").unwrap()); let (n3, _) = elem_move(&n2, &c2[0].range, 0.0, 2.0).unwrap(); assert!(n3.contains("place(dx: 1.5pt, dy: 0pt, it)")); }
    #[test] fn el_move_inline() { let t = "#let f() = {\n  text[가]\n  h(1fr)\n  text[나]\n}"; let c = elem_chain(t, t.find("가").unwrap()); let (n, _) = elem_move(t, &c[0].range, 2.0, 0.0).unwrap(); assert!(n.contains("box(move(dx: 2pt, dy: 0pt, text[가]))")); }
    #[test] fn ins_basic() { let mut x = w("#ex[The |cat][고양이]"); insert(&mut x, "big ").unwrap(); assert_eq!(show(&x), "#ex[The big |cat][고양이]"); }
    #[test] fn ins_escape() { let mut x = w("a |b"); insert(&mut x, "*#[").unwrap(); assert_eq!(x.text, "a \\*\\#\\[b"); }
    #[test] fn ins_after_bold() { let mut x = w("The *cat*| sat"); insert(&mut x, "s").unwrap(); assert_eq!(show(&x), "The #strong[cat]s| sat"); }
    #[test] fn ins_in_bold() { let mut x = w("The *cat|* sat"); insert(&mut x, "s").unwrap(); assert_eq!(show(&x), "The *cats|* sat"); }
    #[test] fn ins_after_call() { let mut x = w("#gp| x"); insert(&mut x, "a").unwrap(); assert_eq!(x.text, "#gp;a x"); }
    #[test] fn ins_str() { let mut x = w("#unit(title: \"듣|다\")"); insert(&mut x, "고 말하").unwrap(); assert_eq!(x.text, "#unit(title: \"듣고 말하다\")"); }
    #[test] fn bs_text() { let mut x = w("가나|다"); backspace(&mut x).unwrap(); assert_eq!(show(&x), "가|다"); }
    #[test] fn bs_into_bold() { let mut x = w("a *bc*| d"); backspace(&mut x).unwrap(); assert_eq!(show(&x), "a *b|* d"); }
    #[test] fn bs_empty_bold() { let mut x = w("a *b|* d"); backspace(&mut x).unwrap(); assert_eq!(show(&x), "a | d"); }
    #[test] fn bs_block_start() { let mut x = w("#ex[|The][가]"); backspace(&mut x).unwrap(); assert_eq!(show(&x), "#ex[|The][가]"); }
    #[test] fn bs_par() { let mut x = w("가\n\n|나"); backspace(&mut x).unwrap(); assert_eq!(show(&x), "가|나"); }
    #[test] fn del_fwd() { let mut x = w("a| *bc* d"); delete(&mut x).unwrap(); assert_eq!(show(&x), "a|#strong[bc] d"); }
    #[test] fn bold_on() { let mut x = w("The {cat} sat"); toggle(&mut x, Style::Bold, "underline").unwrap(); assert_eq!(x.text, "The *cat* sat"); }
    #[test] fn bold_off() { let mut x = w("The *{cat}* sat"); toggle(&mut x, Style::Bold, "underline").unwrap(); assert_eq!(x.text, "The cat sat"); }
    #[test] fn bold_part_off() { let mut x = w("*{big} cat*"); toggle(&mut x, Style::Bold, "underline").unwrap(); assert_eq!(x.text, "big *cat*"); }
    #[test] fn bold_off_weight() { let mut x = w("#text(17pt, weight: \"bold\")[{2028 LEET}]"); toggle(&mut x, Style::Bold, "underline").unwrap(); assert_eq!(x.text, "#text(17pt)[2028 LEET]"); }
    #[test] fn bold_off_weight_first() { let mut x = w("#text(weight: \"bold\", fill: navy)[{로준생}]"); toggle(&mut x, Style::Bold, "underline").unwrap(); assert_eq!(x.text, "#text(fill: navy)[로준생]"); }
    #[test] fn bold_off_weight_solo() { let mut x = w("a #text(weight: \"bold\")[{b}] c"); toggle(&mut x, Style::Bold, "underline").unwrap(); assert_eq!(x.text, "a b c"); }
    #[test] fn bold_off_weight_num() { let mut x = w("a #text(weight: 700)[{b} c] d"); toggle(&mut x, Style::Bold, "underline").unwrap(); assert_eq!(x.text, "a b #text(weight: 700)[c] d"); }
    #[test] fn bold_off_weight_part() { let mut x = w("#text(10pt, weight: \"bold\")[가설이 {거짓}이면]"); toggle(&mut x, Style::Bold, "underline").unwrap(); assert_eq!(x.text, "#text(10pt, weight: \"bold\")[가설이 #text(weight: \"regular\")[거짓]이면]"); }
    #[test] fn bold_on_regular() { let mut x = w("#text(10pt, weight: \"bold\")[가설이 #text(weight: \"regular\")[{거짓}]이면]"); toggle(&mut x, Style::Bold, "underline").unwrap(); assert_eq!(x.text, "#text(10pt, weight: \"bold\")[가설이 거짓이면]"); }
    #[test] fn bold_state_weight() {
        let t = "#text(10pt, weight: \"bold\")[가 #text(weight: \"regular\")[나] 다] *라 #text(weight: \"regular\")[마]*";
        let at = |c: &str| { let p = t.find(c).unwrap(); fmt_state(t, p, p + c.len())[0] };
        assert_eq!([at("가"), at("나"), at("다"), at("라"), at("마")], [true, false, true, true, false]);
    }
    #[test] fn bold_strip_keeps() { let mut x = w("#sub[{내 상식은}?]"); bold_strip(&mut x, true).unwrap(); assert_eq!(x.text, "#sub[내 상식은?]"); bold_wrap(&mut x, true).unwrap(); assert_eq!(x.text, "#sub[#text(weight: \"regular\")[내 상식은]?]"); }
    #[test] fn peel_emph() {
        let mut x = w("효과가 있다. _#text(weight: \"regular\")[{단}]_...");
        assert_eq!(peel_kinds(&x.text, x.sel().unwrap().0, x.sel().unwrap().1), vec!["emph".to_string()]);
        peel(&mut x, "emph").unwrap(); assert_eq!(x.text, "효과가 있다. #text(weight: \"regular\")[단]...");
        bold_strip(&mut x, false).unwrap(); assert_eq!(x.text, "효과가 있다. 단...");
    }
    #[test] fn peel_part() { let mut x = w("_다른 {사람}_이 탁구"); peel(&mut x, "emph").unwrap(); assert_eq!(x.text, "_다른_ 사람이 탁구"); }
    #[test] fn peel_part_break() { let mut x = w("_가 \\ {나}_"); peel(&mut x, "emph").unwrap(); assert_eq!(x.text, "_가 \\ _나"); }
    #[test] fn peel_two() { let mut x = w("{_무엇을_, _어떤_}"); peel(&mut x, "emph").unwrap(); assert_eq!(x.text, "무엇을, 어떤"); }
    #[test] fn peel_order() { let t = "#key[*가 #kw[나] 다*]"; let p = t.find("나").unwrap(); assert_eq!(peel_kinds(t, p, p + 3), vec!["kw".to_string(), "strong".to_string()]); }
    #[test] fn size_wrap() {
        let mut x = w("가 {나다} 라"); text_size(&mut x, 10.0, 12.0).unwrap(); assert_eq!(x.text, "가 #text(size: 12pt)[나다] 라");
        // 다시: 감싼 꾸밈의 값을 고친다
        text_size(&mut x, 12.0, 13.5).unwrap(); assert_eq!(x.text, "가 #text(size: 13.5pt)[나다] 라");
    }
    #[test] fn size_args() {
        let mut x = w("#text(9pt, weight: \"bold\")[{제목}]"); text_size(&mut x, 9.0, 11.0).unwrap(); assert_eq!(x.text, "#text(11pt, weight: \"bold\")[제목]");
        let mut x = w("#text(size: 0.9em)[{작게}]"); text_size(&mut x, 9.0, 10.0).unwrap(); assert_eq!(x.text, "#text(size: 1em)[작게]");
        let mut x = w("#text(weight: \"bold\")[{제목}]"); text_size(&mut x, 10.0, 12.0).unwrap(); assert_eq!(x.text, "#text(size: 12pt, weight: \"bold\")[제목]");
        // 일부만 고르면 안쪽을 감싼다
        let mut x = w("#text(9pt)[가 {나} 다]"); text_size(&mut x, 9.0, 12.0).unwrap(); assert_eq!(x.text, "#text(9pt)[가 #text(size: 12pt)[나] 다]");
    }
    #[test] fn fill_set() {
        let mut x = w("가 {나} 다"); text_fill(&mut x, Some("#c00000")).unwrap(); assert_eq!(x.text, "가 #text(fill: rgb(\"#c00000\"))[나] 다");
        text_fill(&mut x, Some("#1f6fb2")).unwrap(); assert_eq!(x.text, "가 #text(fill: rgb(\"#1f6fb2\"))[나] 다");
        text_fill(&mut x, None).unwrap(); assert_eq!(x.text, "가 나 다");
        let mut x = w("#text(13pt, weight: \"bold\", fill: navy)[{제목}]"); text_fill(&mut x, Some("#c00000")).unwrap(); assert_eq!(x.text, "#text(13pt, weight: \"bold\", fill: rgb(\"#c00000\"))[제목]");
        text_fill(&mut x, None).unwrap(); assert_eq!(x.text, "#text(13pt, weight: \"bold\")[제목]");
        // 자리로 준 색
        let mut x = w("#text(white, size: 12pt)[{1}]"); text_fill(&mut x, Some("#000000")).unwrap(); assert_eq!(x.text, "#text(rgb(\"#000000\"), size: 12pt)[1]");
        let mut x = w("#let ink = rgb(\"#123456\")\n#text(ink)[{가}]"); text_fill(&mut x, Some("#000000")).unwrap(); assert_eq!(x.text, "#let ink = rgb(\"#123456\")\n#text(rgb(\"#000000\"))[가]");
    }
    #[test] fn bold_midword() { let mut x = w("un{believ}able"); toggle(&mut x, Style::Bold, "underline").unwrap(); assert_eq!(x.text, "un#strong[believ]able"); }
    #[test] fn bold_hangul() { let mut x = w("청중{관중}독자"); toggle(&mut x, Style::Bold, "underline").unwrap(); assert_eq!(x.text, "청중*관중*독자"); }
    #[test] fn under_on() { let mut x = w("The {cat} sat"); toggle(&mut x, Style::Under, "underline").unwrap(); assert_eq!(x.text, "The #underline[cat] sat"); }
    #[test] fn under_off() { let mut x = w("The #u[{cat}] sat #u[x]"); toggle(&mut x, Style::Under, "underline").unwrap(); assert_eq!(x.text, "The cat sat #u[x]"); }
    #[test] fn under_keep_bold() { let mut x = w("The {*cat* sat}"); toggle(&mut x, Style::Under, "underline").unwrap(); assert_eq!(x.text, "The #underline[*cat* sat]"); }
    #[test] fn bold_off_nested() { let mut x = w("a *#underline[{rose}]* b"); toggle(&mut x, Style::Bold, "underline").unwrap(); assert_eq!(x.text, "a #underline[rose] b"); }
    #[test] fn under_in_bold() { let mut x = w("*big {cat} sat*"); toggle(&mut x, Style::Under, "underline").unwrap(); assert_eq!(x.text, "*big #underline[cat] sat*"); }
    #[test] fn bold_across() { let mut x = w("a {b *c* d} e"); toggle(&mut x, Style::Bold, "underline").unwrap(); assert_eq!(x.text, "a *b c d* e"); }
    #[test] fn bs_hangul_str() { let mut x = w("#sub(\"낱말 익히|기\")"); backspace(&mut x).unwrap(); assert_eq!(x.text, "#sub(\"낱말 익기\")"); }
    #[test] fn paste_lines() { let mut x = w("가|나"); insert(&mut x, "A\nB\n\nC").unwrap(); assert_eq!(x.text, "가A\\ B\n\nC나"); }
    #[test] fn enter_plain() { let mut x = w("가|나"); enter(&mut x).unwrap(); assert_eq!(show(&x), "가\n\n|나"); }
    #[test] fn enter_end_type() {
        // 상자 안 마지막 문단 끝에서 Enter를 친 뒤에도 글자가 들어가야 한다
        let mut x = w("#key[\n  가.|\n]"); enter(&mut x).unwrap(); insert(&mut x, "나").unwrap(); assert_eq!(show(&x), "#key[\n  가.\n\n나|\n]");
        let mut x = w("가.|\n\n다."); enter(&mut x).unwrap(); insert(&mut x, "나").unwrap(); assert_eq!(show(&x), "가.\n\n나|\n\n다.");
        assert!(blank_gap("가.\n\n\n\n다.", 4, 6) && !blank_gap("가. 나\n\n다", 4, 10) && !blank_gap("가.\n다", 4, 5));
        assert!(blank_gap("#text(9pt)[가.]\n\n#text(9pt)[]", 15, 29));
    }
    #[test] fn enter_bold() { let mut x = w("*가|나*"); enter(&mut x).unwrap(); assert_eq!(show(&x), "*가*\n\n*|나*"); }
    #[test] fn lb() { let mut x = w("가|나"); linebreak(&mut x).unwrap(); assert_eq!(show(&x), "가\\ |나"); }
    #[test] fn tab_bs() { let mut x = w("가|나"); tab(&mut x, 12.0).unwrap(); assert_eq!(show(&x), "가#h(12.0pt)|나"); backspace(&mut x).unwrap(); assert_eq!(show(&x), "가|나"); }
    #[test] fn hang() {
        let mut x = w("앞\n\n가|나 다\n\n뒤"); hanging(&mut x, 1).unwrap(); assert_eq!(x.text, "앞\n\n#par(hanging-indent: 1em)[가나 다]\n\n뒤");
        hanging(&mut x, 1).unwrap(); assert_eq!(x.text, "앞\n\n#par(hanging-indent: 2em)[가나 다]\n\n뒤");
        hanging(&mut x, -1).unwrap(); hanging(&mut x, -1).unwrap(); assert_eq!(x.text, "앞\n\n가나 다\n\n뒤");
    }
    #[test] fn del_sel_keep_marks() { let mut x = w("a *b{c* d}e"); delete_sel(&mut x).unwrap(); assert_eq!(show(&x), "a #strong[b|]e"); }
    #[test] fn del_sel_cross() { let mut x = w("#ex[a{b][c}d]"); delete_sel(&mut x).unwrap(); assert_eq!(show(&x), "#ex[a|][d]"); }
    #[test] fn del_sel_items() { let mut x = w("가{나\n\n#ex[a][b]\n\n#let k = 1\n\n다}라"); delete_sel(&mut x).unwrap(); assert_eq!(show(&x), "가|\n\n#let k = 1\n\n라"); }
    #[test] fn del_sel_str_mixed() { let mut x = w("#unit(title: \"듣{다\")[가}나]"); assert!(delete_sel(&mut x).is_err()); }
    // 글줄에 낀 템플릿 꾸밈(#lk 등)은 굵게처럼 그냥 넘어가며 지운다
    #[test] fn bs_into_custom() { let mut x = w("a #lk(\"a\")[bc]| d"); backspace(&mut x).unwrap(); assert_eq!(show(&x), "a #lk(\"a\")[b|] d"); }
    #[test] fn bs_out_of_custom() { let mut x = w("a #lk(\"a\")[|bc] d"); backspace(&mut x).unwrap(); assert_eq!(show(&x), "a|#lk(\"a\")[bc] d"); }
    #[test] fn bs_empty_custom() { let mut x = w("a #lk(\"a\")[b|] d"); backspace(&mut x).unwrap(); assert_eq!(show(&x), "a | d"); assert_eq!(x.note.as_deref(), Some("lk")); }
    #[test] fn bs_sweep() {
        let mut x = w("#ex(0)[I #lk(\"a\")[saw] #pt[the] cat.|][나]");
        let mut n = 0;
        while !x.edge && n < 60 { backspace(&mut x).unwrap(); n += 1; }
        assert_eq!(show(&x), "#ex(0)[|][나]");
    }
    // 글 상자의 가장자리: 지울 것이 없으면 가장자리라고 알린다(커서를 지면에서 옮긴다)
    #[test] fn bs_edge_block() { let mut x = w("#ex[The][|가]"); backspace(&mut x).unwrap(); assert!(x.edge); assert_eq!(show(&x), "#ex[The][|가]"); }
    #[test] fn bs_edge_after_block() { let mut x = w("#ex[a][b]\n\n|가"); backspace(&mut x).unwrap(); assert!(x.edge); assert_eq!(x.text, "#ex[a][b]\n\n가"); }
    #[test] fn bs_edge_set() { let mut x = w("가\n\n#set text(red)\n\n|나"); backspace(&mut x).unwrap(); assert!(x.edge); }
    #[test] fn bs_join_par_keeps() { let mut x = w("가 #lk(\"a\")[나]\n\n|다"); backspace(&mut x).unwrap(); assert_eq!(show(&x), "가 #lk(\"a\")[나]|다"); }
    // 속이 다 빈 요소는 한 번 더 지우면 통째로 사라진다
    #[test] fn bs_hollow() { let mut x = w("가\n\n#ex(0)[][|][#nt(\"문맥\")[]]\n\n나"); backspace(&mut x).unwrap(); assert_eq!(show(&x), "가\n\n|\n나"); assert_eq!(x.note.as_deref(), Some("ex")); }
    #[test] fn bs_not_hollow() { let mut x = w("#ex(0)[a][|]"); backspace(&mut x).unwrap(); assert!(x.edge); assert_eq!(x.text, "#ex(0)[a][]"); }
    // 글이 없는 낱개 요소는 통째로
    #[test] fn bs_atom_call() { let mut x = w("가 #pos(\"명\")| 나"); backspace(&mut x).unwrap(); assert_eq!(show(&x), "가 | 나"); assert_eq!(x.note.as_deref(), Some("pos")); }
    #[test] fn bs_atom_ident() { let mut x = w("가 #gp;|나"); backspace(&mut x).unwrap(); assert_eq!(show(&x), "가 |나"); }
    #[test] fn del_atom_call() { let mut x = w("가 |#pos(\"명\") 나"); delete(&mut x).unwrap(); assert_eq!(show(&x), "가 | 나"); }
    #[test] fn del_into_custom() { let mut x = w("a |#lk(\"a\")[bc] d"); delete(&mut x).unwrap(); assert_eq!(show(&x), "a #lk(\"a\")[|c] d"); }
    #[test] fn del_edge_block() { let mut x = w("#ex[The|][가]"); delete(&mut x).unwrap(); assert!(x.edge); }
    // 지운 자리에서 앞뒤가 달라붙어 뜻이 바뀌지 않게
    #[test] fn bs_glue_ident() { let mut x = w("#gp |다"); backspace(&mut x).unwrap(); assert_eq!(show(&x), "#gp;|다"); }
    #[test] fn bs_line_marker() { let mut x = w("가\n\nA|- b"); backspace(&mut x).unwrap(); assert_eq!(show(&x), "가\n\n|\\- b"); }
    // 낱말 지우기
    #[test] fn bs_word() { let mut x = w("The big cat|"); del_word(&mut x, false).unwrap(); assert_eq!(show(&x), "The big |"); del_word(&mut x, false).unwrap(); assert_eq!(show(&x), "The |"); }
    #[test] fn del_word_fwd() { let mut x = w("The |big cat"); del_word(&mut x, true).unwrap(); assert_eq!(show(&x), "The |cat"); }
    #[test] fn bs_word_str() { let mut x = w("#sub(\"낱말 익히기|\")"); del_word(&mut x, false).unwrap(); assert_eq!(x.text, "#sub(\"낱말 \")"); }
    // 선택 위에 입력하면 꾸밈을 물려받는다
    #[test] fn type_over_bold() { let mut x = w("The *{cat}* sat"); insert(&mut x, "dog").unwrap(); assert_eq!(show(&x), "The *dog|* sat"); }
    #[test] fn type_over_custom() { let mut x = w("a #lk(\"a\")[{bc}] d"); insert(&mut x, "xy").unwrap(); assert_eq!(show(&x), "a #lk(\"a\")[xy|] d"); }
    // 기울임·취소선·정렬
    #[test] fn italic_on() { let mut x = w("The {cat} sat"); toggle(&mut x, Style::Italic, "underline").unwrap(); assert_eq!(x.text, "The _cat_ sat"); }
    #[test] fn italic_off() { let mut x = w("The _{cat}_ sat"); toggle(&mut x, Style::Italic, "underline").unwrap(); assert_eq!(x.text, "The cat sat"); }
    #[test] fn italic_midword() { let mut x = w("un{believ}able"); toggle(&mut x, Style::Italic, "underline").unwrap(); assert_eq!(x.text, "un#emph[believ]able"); }
    #[test] fn strike_on_off() { let mut x = w("The {cat} sat"); toggle(&mut x, Style::Strike, "underline").unwrap(); assert_eq!(x.text, "The #strike[cat] sat"); x.anchor = 12; x.caret = 15; toggle(&mut x, Style::Strike, "underline").unwrap(); assert_eq!(x.text, "The cat sat"); }
    #[test] fn bold_cross() { let mut x = w("#ex[a {b][c} d]"); toggle(&mut x, Style::Bold, "underline").unwrap(); assert_eq!(x.text, "#ex[a *b*][*c* d]"); }
    #[test] fn align_par() {
        let mut x = w("앞\n\n가|나 다\n\n뒤"); align(&mut x, Some("center")).unwrap(); assert_eq!(x.text, "앞\n\n#align(center)[가나 다]\n\n뒤");
        assert_eq!(align_state(&x.text, x.caret).as_deref(), Some("center"));
        align(&mut x, Some("right")).unwrap(); assert_eq!(x.text, "앞\n\n#align(right)[가나 다]\n\n뒤");
        align(&mut x, None).unwrap(); assert_eq!(x.text, "앞\n\n가나 다\n\n뒤");
    }
    #[test] fn ranges() {
        let t = "#ex[The cat.\n\nA dog.][나]";
        assert_eq!(para_range(t, 6), Some((4, 12)));
        assert_eq!(block_range(t, 6), Some((4, 20)));
        // 줄을 달리한 요소들은 저마다 따로 선 덩어리: 문단·글 상자를 그 안에서 찾는다
        let t = "#head(\"가\")\n#prose[\n첫 문단이다.\n\n둘째 문단.\n]";
        let p = t.find("둘째").unwrap() + 3;
        assert_eq!(para_range(t, p).map(|(s, e)| &t[s..e]), Some("둘째 문단."));
        assert_eq!(block_range(t, p).map(|(s, e)| &t[s..e]), Some("첫 문단이다.\n\n둘째 문단."));
    }
    // 글이 다 지워져 혼자 남은 낱개 요소도 이어서 지워진다
    #[test] fn bs_lone_atom() {
        let mut x = w("#entry(mean: [#pos(\"명\") |])");
        backspace(&mut x).unwrap(); assert_eq!(show(&x), "#entry(mean: [#pos(\"명\")|])");
        backspace(&mut x).unwrap(); assert_eq!(show(&x), "#entry(mean: [|])");
    }
    const STACK: &str = "#stack(spacing: 8pt,\n  check[가],\n  check[나],\n  check[다],\n)";
    #[test] fn bs_item_empty() {
        let mut x = w(&STACK.replace("[나]", "[|]")); backspace(&mut x).unwrap();
        assert_eq!(show(&x), "#stack(spacing: 8pt,\n  check[가],\n|  check[다],\n)"); assert_eq!(x.note.as_deref(), Some("check"));
        let mut x = w(&STACK.replace("[나]", "[|]")); delete(&mut x).unwrap();
        assert_eq!(show(&x), "#stack(spacing: 8pt,\n  check[가],\n|  check[다],\n)");
    }
    #[test] fn bs_item_before() {
        // 빈 항목 다음 항목의 맨 앞에서 Backspace: 빈 항목이 사라지고 커서는 제자리
        let mut x = w(&STACK.replace("[나]", "[]").replace("[다]", "[|다]")); backspace(&mut x).unwrap();
        assert_eq!(show(&x), "#stack(spacing: 8pt,\n  check[가],\n  check[|다],\n)"); assert_eq!(x.note.as_deref(), Some("check"));
    }
    #[test] fn bs_item_after() {
        // 빈 항목 앞 항목의 맨 끝에서 Delete
        let mut x = w(&STACK.replace("[나]", "[]").replace("[가]", "[가|]")); delete(&mut x).unwrap();
        assert_eq!(show(&x), "#stack(spacing: 8pt,\n  check[가|],\n  check[다],\n)");
    }
    #[test] fn bs_item_last() {
        let mut x = w("#stack(check[가], check[|])"); backspace(&mut x).unwrap(); assert_eq!(show(&x), "#stack(check[가]|)");
        let mut x = w("#stack(check[|], check[가])"); backspace(&mut x).unwrap(); assert_eq!(show(&x), "#stack(|check[가])");
    }
    #[test] fn bs_item_sink() {
        let t = "#let row(fit: false, ..cards) = grid(..cards)\n#row(card[가], card[|])";
        let mut x = w(t); backspace(&mut x).unwrap(); assert_eq!(show(&x), "#let row(fit: false, ..cards) = grid(..cards)\n#row(card[가]|)");
    }
    #[test] fn bs_item_keep() {
        // 표·격자의 칸과, 인자 수가 정해진 함수의 인자는 빼지 않는다
        for t in ["#grid(columns: 2, cell[가], cell[|], cell[다])", "#let f(a, b) = a + b\n#f(text[|], text[나])", "#stack(img(\"a.png\")[], check[|다])"] {
            let mut x = w(t); backspace(&mut x).unwrap(); assert_eq!(show(&x), t); assert!(x.edge);
        }
    }
    #[test] fn el_del_item() {
        let c = elem_chain(STACK, STACK.find("나").unwrap()); assert_eq!(c[0].name, "check");
        assert_eq!(elem_delete(STACK, &c[0].range).unwrap(), "#stack(spacing: 8pt,\n  check[가],\n  check[다],\n)");
    }
    #[test] fn bs_dead_str() {
        let mut x = w("가\n\n#sub(\"기|\")\n\n나");
        backspace(&mut x).unwrap(); assert_eq!(show(&x), "가\n\n#sub(\"|\")\n\n나");
        backspace(&mut x).unwrap(); assert_eq!(show(&x), "가\n\n|\n나"); assert_eq!(x.note.as_deref(), Some("sub"));
        let mut y = w("#unit(title: \"|\", root: \"audi\")"); backspace(&mut y).unwrap(); assert!(y.edge);
    }
    // 상자·표의 너비
    fn call(t: &str, name: &str) -> Range<usize> { let s = t.find(name).unwrap(); let node = tree(t); let root = LinkedNode::new(&node); let mut n = root.leaf_at(s + 1, Side::Before).unwrap(); while n.kind() != SyntaxKind::FuncCall { n = n.parent().unwrap().clone(); } n.range() }
    const MM: f32 = 2.834_645_7;
    /// 한 파일 안에서: 상자 `call`을 끌고(부른 자리는 `site` 글자가 있는 곳), 고친 본문을 돌려준다.
    fn bx(t: &str, name: &str, site: &str, local: bool, col: Option<usize>, dx: f32, width: f32, cols: &[f32]) -> (String, bool) {
        let e = box_resize(t, &call(t, name), Some((t, t.find(site).unwrap())), local, false, col, dx, Some(width), cols).unwrap();
        let all: Vec<Edit> = e.def.iter().chain(e.site.iter()).cloned().collect();
        let out = apply_edits(t, &all);
        assert!(!bad(&tree(&out)), "{out}");
        (out, e.local)
    }
    const KEY: &str = "#let key(body) = block(width: 100%, fill: gray, body)\n#key[가]\n#key[나]";
    #[test] fn bx_local_new() {
        let (t, local) = bx(KEY, "block", "나", true, None, -50.0, 200.0, &[]);
        assert_eq!(t, "#let key(body, width: 100%) = block(width: width, fill: gray, body)\n#key[가]\n#key(width: 75%)[나]"); assert!(local);
        // 한 번 더: 이미 매개변수가 있다
        let (t2, _) = bx(&t, "block", "나", true, None, -50.0, 150.0, &[]);
        assert_eq!(t2, "#let key(body, width: 100%) = block(width: width, fill: gray, body)\n#key[가]\n#key(width: 50%)[나]");
        // 다른 곳
        let (t3, _) = bx(&t2, "block", "가", true, None, -100.0, 200.0, &[]);
        assert_eq!(t3, "#let key(body, width: 100%) = block(width: width, fill: gray, body)\n#key(width: 50%)[가]\n#key(width: 50%)[나]");
        // 모두(Shift): 기본값을 고친다
        let (t4, local) = bx(&t3, "block", "가", false, None, -100.0, 200.0, &[]);
        assert_eq!(t4, "#let key(body, width: 50%) = block(width: width, fill: gray, body)\n#key(width: 50%)[가]\n#key(width: 50%)[나]"); assert!(!local);
    }
    #[test] fn bx_local_all() { let (t, local) = bx(KEY, "block", "나", false, None, -50.0, 200.0, &[]); assert_eq!(t, KEY.replace("100%", "75%")); assert!(!local); }
    #[test] fn bx_local_auto() {
        let t = "#let card(body) = box(stroke: 1pt, body)\n#card[가] #card[나]";
        let (o, _) = bx(t, "box", "나", true, None, 10.0 * MM, 20.0 * MM, &[]);
        assert_eq!(o, "#let card(body, width: auto) = box(width: width, stroke: 1pt, body)\n#card[가] #card(width: 30mm)[나]");
    }
    #[test] fn bx_local_args() {
        let t = "#let qr(path, len: none) = stack(image(path, width: 17mm), [가])\n#qr(\"a.svg\", len: 3)";
        let (o, _) = bx(t, "image", "a.svg", true, None, 3.0 * MM, 17.0 * MM, &[]);
        assert_eq!(o, "#let qr(path, len: none, width: 17mm) = stack(image(path, width: width), [가])\n#qr(\"a.svg\", len: 3, width: 20mm)");
    }
    #[test] fn bx_local_cols() {
        let t = "#let check(t) = grid(columns: (12pt, 1fr), box(), t)\n#check[가]\n#check[나]";
        let (o, _) = bx(t, "grid", "나", true, Some(0), 6.0, 300.0, &[12.0, 288.0]);
        assert_eq!(o, "#let check(t, columns: (12pt, 1fr)) = grid(columns: columns, box(), t)\n#check[가]\n#check(columns: (18pt, 1fr))[나]");
    }
    #[test] fn bx_param() {
        // 값이 매개변수: 이 곳만이면 부른 자리에, 모두면 기본값에
        let t = "#let card(path, size: 18mm) = image(path, width: size)\n#card(\"a.svg\")\n#card(\"b.svg\", size: 20mm)";
        assert_eq!(bx(t, "image", "a.svg", true, None, 2.0 * MM, 18.0 * MM, &[]).0, t.replace("#card(\"a.svg\")", "#card(\"a.svg\", size: 20mm)"));
        assert_eq!(bx(t, "image", "b.svg", true, None, 2.0 * MM, 20.0 * MM, &[]).0, t.replace("size: 20mm", "size: 22mm"));
        assert_eq!(bx(t, "image", "a.svg", false, None, 2.0 * MM, 18.0 * MM, &[]).0, t.replace("size: 18mm", "size: 20mm"));
    }
    #[test] fn bx_var() {
        // 값이 변수: 그 변수를 정한 곳을 고친다
        let t = "#let w = 60mm\n#block(width: w)[가]\n#block(width: w)[나]";
        assert_eq!(bx(t, "block", "가", true, None, 10.0 * MM, 60.0 * MM, &[]).0, t.replace("60mm", "70mm"));
        let t = "#let cols = (20mm, 1fr)\n#grid(columns: cols, [가], [나])";
        assert_eq!(bx(t, "grid", "가", true, Some(0), 5.0 * MM, 100.0 * MM, &[20.0 * MM, 80.0 * MM]).0, t.replace("20mm", "25mm"));
    }
    #[test] fn bx_plain() {
        // 템플릿이 아니면 전과 같다
        let t = "#block(width: 60mm)[가]"; let (o, local) = bx(t, "block", "가", true, None, 10.0 * MM, 60.0 * MM, &[]); assert_eq!(o, "#block(width: 70mm)[가]"); assert!(!local);
        // 이름 없는 함수(map) 안의 상자는 모두 함께
        let t = "#grid(..([가], [나]).map(a => block(width: 100%, a)))"; assert_eq!(bx(t, "block", "가", true, None, -50.0, 100.0, &[]).0, t.replace("100%", "50%"));
    }
    fn bv(t: &str, name: &str, site: &str, local: bool, row: Option<usize>, dy: f32, height: f32, rows: &[f32]) -> String {
        let e = box_resize(t, &call(t, name), Some((t, t.find(site).unwrap())), local, true, row, dy, Some(height), rows).unwrap();
        let all: Vec<Edit> = e.def.iter().chain(e.site.iter()).cloned().collect();
        let out = apply_edits(t, &all);
        assert!(!bad(&tree(&out)), "{out}");
        out
    }
    #[test] fn bv_height() {
        assert_eq!(bv("#block(width: 60mm)[가]", "block", "가", true, None, 5.0 * MM, 10.0 * MM, &[]), "#block(height: 15mm, width: 60mm)[가]");
        assert_eq!(bv("#box(height: 1.2em, width: 32mm)", "box", "box", true, None, 6.0, 12.0, &[]), "#box(height: 1.8em, width: 32mm)");
        assert_eq!(bv("#rect(height: 20pt)[가]", "rect", "가", true, None, -100.0, 20.0, &[]), "#rect(height: 3pt)[가]");
    }
    #[test] fn bv_rows() {
        let t = "#table(columns: (auto, auto), [가], [나], [다], [라])";
        // 줄 높이가 적혀 있지 않던 표: 끈 줄만 수로 적는다
        assert_eq!(bv(t, "table", "가", true, Some(0), 2.0 * MM, 20.0 * MM, &[8.0 * MM, 8.0 * MM]), "#table(columns: (auto, auto), rows: (10mm, auto), [가], [나], [다], [라])");
        assert_eq!(bv(t, "table", "가", true, None, 2.0 * MM, 20.0 * MM, &[8.0 * MM, 8.0 * MM]), "#table(columns: (auto, auto), rows: (auto, 10mm), [가], [나], [다], [라])");
        let t = "#grid(columns: 2, rows: (10mm, 12mm), [가], [나], [다], [라])";
        assert_eq!(bv(t, "grid", "가", true, Some(1), -2.0 * MM, 22.0 * MM, &[10.0 * MM, 12.0 * MM]), t.replace("12mm", "10mm"));
        // 한 값이 모든 줄에 되풀이되던 표: 줄마다 풀어 적는다
        let t = "#grid(columns: 2, rows: 10mm, [가], [나], [다], [라])";
        assert_eq!(bv(t, "grid", "가", true, Some(0), 5.0 * MM, 20.0 * MM, &[10.0 * MM, 10.0 * MM]), t.replace("rows: 10mm", "rows: (15mm, 10mm)"));
    }
    #[test] fn bv_local() {
        let t = "#let key(body) = block(width: 100%, body)\n#key[가]\n#key[나]";
        assert_eq!(bv(t, "block", "나", true, None, 5.0 * MM, 10.0 * MM, &[]), "#let key(body, height: auto) = block(height: height, width: 100%, body)\n#key[가]\n#key(height: 15mm)[나]");
    }
    #[test] fn img_path() {
        let t = "#image(\"fig/a.png\", width: 40mm)";
        let r = call(t, "image");
        assert_eq!(image_path(t, &r, Some((t, 2))).as_deref(), Some("fig/a.png"));
        let e = image_replace(t, &r, Some((t, 2)), "fig/b.jpg").unwrap(); assert_eq!(apply_edits(t, &e.def), "#image(\"fig/b.jpg\", width: 40mm)");
        // 템플릿 함수의 매개변수로 받은 경로: 부른 자리의 글자를 바꾼다
        let t = "#let card(path, title, len: none) = stack(image(path, width: 17mm), title)\n#card(\"qr/a.svg\", [가])\n#card(\"qr/b.svg\", [나])";
        let r = call(t, "image");
        let at = t.find("나").unwrap();
        assert_eq!(image_path(t, &r, Some((t, at))).as_deref(), Some("qr/b.svg"));
        let e = image_replace(t, &r, Some((t, at)), "qr/c.png").unwrap(); assert!(e.def.is_empty());
        assert_eq!(apply_edits(t, &e.site), t.replace("qr/b.svg", "qr/c.png"));
        // 식으로 만든 경로는 바꿀 수 없다
        let t = "#image(\"fig/\" + name)"; assert!(image_replace(t, &call(t, "image"), Some((t, 2)), "x.png").is_err());
    }
    const TB: &str = "#table(columns: (auto, 1fr), align: center,\n  [가], [나],\n  [다], [라],\n)";
    fn top(t: &str, at: &str, kind: &str) -> String { let (o, c) = table_op(t, t.find(at).unwrap(), kind).unwrap(); format!("{}|{}", &o[..c], &o[c..]) }
    #[test] fn tb_rows() {
        assert_eq!(top(TB, "다", "rowBelow"), "#table(columns: (auto, 1fr), align: center,\n  [가], [나],\n  [다], [라],\n  [|], [],\n)");
        assert_eq!(top(TB, "가", "rowBelow"), "#table(columns: (auto, 1fr), align: center,\n  [가], [나],\n  [|], [],\n  [다], [라],\n)");
        assert_eq!(top(TB, "나", "rowAbove"), "#table(columns: (auto, 1fr), align: center,\n  [], [|],\n  [가], [나],\n  [다], [라],\n)");
        assert_eq!(top(TB, "라", "rowDel"), "#table(columns: (auto, 1fr), align: center,\n  [가], [|나],\n)");
        assert_eq!(top(TB, "가", "rowDel"), "#table(columns: (auto, 1fr), align: center,\n  [|다], [라],\n)");
        // 한 줄에 다 적은 표, 끝에 쉼표가 없는 표
        let t = "#table(columns: 2, [가], [나], [다], [라])";
        assert_eq!(top(t, "다", "rowBelow"), "#table(columns: 2, [가], [나], [다], [라], [|], [])");
        assert_eq!(top(t, "가", "rowAbove"), "#table(columns: 2, [|], [], [가], [나], [다], [라])");
        assert_eq!(top(t, "라", "rowDel"), "#table(columns: 2, [가], [|나])");
        assert_eq!(top(t, "나", "rowDel"), "#table(columns: 2, [다], [|라])");
        // 줄 높이가 줄마다 적힌 표
        let t = "#table(columns: 2, rows: (8mm, auto), [가], [나], [다], [라])";
        assert_eq!(top(t, "가", "rowBelow"), "#table(columns: 2, rows: (8mm, auto, auto), [가], [나], [|], [], [다], [라])");
        assert_eq!(top(t, "가", "rowDel"), "#table(columns: 2, rows: (auto,), [|다], [라])");
    }
    #[test] fn tb_cols() {
        assert_eq!(top(TB, "가", "colRight"), "#table(columns: (auto, auto, 1fr), align: center,\n  [가], [|], [나],\n  [다], [], [라],\n)");
        assert_eq!(top(TB, "라", "colRight"), "#table(columns: (auto, 1fr, 1fr), align: center,\n  [가], [나], [],\n  [다], [라], [|],\n)");
        assert_eq!(top(TB, "다", "colLeft"), "#table(columns: (auto, auto, 1fr), align: center,\n  [], [가], [나],\n  [|], [다], [라],\n)");
        assert_eq!(top(TB, "나", "colDel"), "#table(columns: (auto,), align: center,\n  [|가],\n  [다],\n)");
        assert_eq!(top(TB, "다", "colDel"), "#table(columns: (1fr,), align: center,\n  [나],\n  [|라],\n)");
        let t = "#grid(columns: 3, [가], [나], [다], [라], [마], [바])";
        assert_eq!(top(t, "마", "colDel"), "#grid(columns: 2, [가], [다], [라], [|바])");
        assert_eq!(top(t, "바", "colRight"), "#grid(columns: 4, [가], [나], [다], [], [라], [마], [바], [|])");
    }
    #[test] fn tb_guard() {
        // 마지막 줄·칸, 코드로 만든 표, 합친 칸
        assert!(table_op("#table(columns: 2, [가], [나])", 20, "rowDel").is_err());
        assert!(table_op("#table([가], [나])", 9, "colDel").is_err());
        let t = "#table(columns: 2, ..rows.map(r => [가]))"; assert!(table_op(t, t.find("가").unwrap(), "rowBelow").is_err());
        let t = "#table(columns: 2, table.cell(colspan: 2)[가], [나], [다])"; assert!(table_op(t, t.find("나").unwrap(), "rowBelow").is_err());
        // 빈 칸에 글자를 넣을 수 있다
        let (o, c) = table_op(TB, TB.find("다").unwrap(), "rowBelow").unwrap();
        let mut x = Work::new(o, c, c); insert(&mut x, "새").unwrap(); assert!(x.text.contains("[새], [],"));
        let w = table_where(&x.text, x.caret).unwrap(); assert_eq!((w.col, w.row, w.ncols, w.nrows, w.empty), (0, 2, 2, 3, false));
        assert_eq!(table_cell(&x.text, &w.call, 1, 2).map(|v| v.1), Some(true));
    }
    #[test] fn fn_body() {
        let t = "#let quote(body, by) = block(width: 100%, inset: 2mm)[#body \\ #by]\n#let sec(n) = { v(1em); block(n) }\n#quote[가][나]\n#quote[다][라]";
        assert_eq!(fn_sized_body(t, "quote").map(|r| &t[r.start..r.start + 5]), Some("block"));
        assert!(fn_sized_body(t, "sec").is_none() && fn_sized_body(t, "none").is_none());
        assert_eq!(count_calls(t, "quote"), 2);
    }
    #[test] fn bx_map_pos() {
        let e: Vec<Edit> = vec![(5..5, "ab".into()), (10..12, "x".into())];
        assert_eq!([map_pos(&e, 3), map_pos(&e, 5), map_pos(&e, 6), map_pos(&e, 11), map_pos(&e, 12), map_pos(&e, 20)], [3, 5, 8, 12, 13, 21]);
        assert_eq!(apply_edits("0123456789ABCDEF", &e), "01234ab56789xCDEF");
    }
    #[test] fn rs_width_mm() { let t = "#block(width: 60mm, inset: 2mm)[가]"; assert_eq!(elem_resize(t, &call(t, "block"), None, 10.0 * MM, Some(60.0 * MM), &[]).unwrap(), "#block(width: 70mm, inset: 2mm)[가]"); }
    #[test] fn rs_width_pct() { let t = "#block(width: 80%)[가]"; assert_eq!(elem_resize(t, &call(t, "block"), None, -50.0, Some(200.0), &[]).unwrap(), "#block(width: 60%)[가]"); }
    #[test] fn rs_width_new() {
        let t = "#block(inset: 2mm)[가]"; assert_eq!(elem_resize(t, &call(t, "block"), None, 10.0 * MM, Some(30.0 * MM), &[]).unwrap(), "#block(width: 40mm, inset: 2mm)[가]");
        let t = "#box[가]"; assert_eq!(elem_resize(t, &call(t, "box"), None, 0.0, Some(20.0 * MM), &[]).unwrap(), "#box(width: 20mm)[가]");
        let t = "#rect()"; assert_eq!(elem_resize(t, &call(t, "rect"), None, 0.0, Some(20.0 * MM), &[]).unwrap(), "#rect(width: 20mm)");
    }
    #[test] fn rs_width_expr() { let t = "#let f(w) = block(width: w)[가]"; let n = elem_resize(t, &call(t, "block"), None, 3.0, Some(100.0), &[]).unwrap(); assert_eq!(n, "#let f(w) = block(width: w + 3pt)[가]"); assert_eq!(elem_resize(&n, &call(&n, "block"), None, 2.0, Some(103.0), &[]).unwrap(), "#let f(w) = block(width: w + 5pt)[가]"); }
    #[test] fn rs_width_min() { let t = "#box(width: 10pt)[가]"; assert_eq!(elem_resize(t, &call(t, "box"), None, -500.0, Some(10.0), &[]).unwrap(), "#box(width: 6pt)[가]"); }
    #[test] fn rs_col_pair() { let t = "#table(columns: (20mm, 30mm, auto), [가], [나], [다])"; assert_eq!(elem_resize(t, &call(t, "table"), Some(0), 5.0 * MM, None, &[20.0 * MM, 30.0 * MM, 40.0]).unwrap(), "#table(columns: (25mm, 25mm, auto), [가], [나], [다])"); }
    #[test] fn rs_col_fr_right() { let t = "#grid(columns: (8mm, 1fr), [=], [가])"; assert_eq!(elem_resize(t, &call(t, "grid"), Some(0), 2.0 * MM, None, &[8.0 * MM, 300.0]).unwrap(), "#grid(columns: (10mm, 1fr), [=], [가])"); }
    #[test] fn rs_col_fr_left() { let t = "#grid(columns: (1fr, 30mm), [가], [나])"; assert_eq!(elem_resize(t, &call(t, "grid"), Some(0), 5.0 * MM, None, &[200.0, 30.0 * MM]).unwrap(), "#grid(columns: (1fr, 25mm), [가], [나])"); }
    #[test] fn rs_col_fr_both() { let t = "#grid(columns: (1fr, 1fr), [가], [나])"; assert_eq!(elem_resize(t, &call(t, "grid"), Some(0), 50.0, None, &[100.0, 100.0]).unwrap(), "#grid(columns: (1.5fr, 0.5fr), [가], [나])"); }
    #[test] fn rs_col_auto_right() { let t = "#grid(columns: (auto, auto), [가], [나])"; assert_eq!(elem_resize(t, &call(t, "grid"), Some(0), 20.0 * MM, None, &[10.0 * MM, 5.0 * MM]).unwrap(), "#grid(columns: (30mm, auto), [가], [나])"); }
    #[test] fn rs_col_clamp() { let t = "#grid(columns: (20pt, 30pt), [가], [나])"; assert_eq!(elem_resize(t, &call(t, "grid"), Some(0), 500.0, None, &[20.0, 30.0]).unwrap(), "#grid(columns: (44pt, 6pt), [가], [나])"); }
    #[test] fn rs_col_int() { let t = "#grid(columns: 3, [가], [나], [다])"; assert_eq!(elem_resize(t, &call(t, "grid"), Some(1), 10.0 * MM, None, &[20.0 * MM, 20.0 * MM, 20.0 * MM]).unwrap(), "#grid(columns: (auto, 30mm, auto), [가], [나], [다])"); }
    #[test] fn rs_col_pct() { let t = "#grid(columns: (30%, 1fr), [가], [나])"; assert_eq!(elem_resize(t, &call(t, "grid"), Some(0), 30.0, None, &[90.0, 210.0]).unwrap(), "#grid(columns: (40%, 1fr), [가], [나])"); }
    #[test] fn rs_edge() {
        let t = "#table(columns: (20mm, 30mm), [가], [나])"; assert_eq!(elem_resize(t, &call(t, "table"), None, 10.0 * MM, None, &[20.0 * MM, 30.0 * MM]).unwrap(), "#table(columns: (20mm, 40mm), [가], [나])");
        let t = "#table(columns: (20mm, 1fr), [가], [나])"; assert!(elem_resize(t, &call(t, "table"), None, 10.0, None, &[56.7, 300.0]).is_err());
        let t = "#table(columns: cols, [가], [나])"; assert!(elem_resize(t, &call(t, "table"), Some(0), 10.0, None, &[56.7, 300.0]).is_err());
    }
    #[test] fn rs_kind() { let t = "#grid(columns: 2)[가] #block[나] #strong[다]"; assert_eq!(resize_kind(t, &call(t, "grid")), Some(("grid".into(), true))); assert_eq!(resize_kind(t, &call(t, "block")), Some(("block".into(), false))); assert_eq!(resize_kind(t, &call(t, "strong")), None); }
    #[test] fn bs_edge_stacked() { let mut x = w("#head(\"가\")\n#fsub[|나]"); backspace(&mut x).unwrap(); assert!(x.edge); assert_eq!(x.text, "#head(\"가\")\n#fsub[나]"); }
    #[test] fn bs_same_line_wraps() { let mut x = w("#lk(\"a\")[가] #pt[|나]"); backspace(&mut x).unwrap(); assert_eq!(show(&x), "#lk(\"a\")[가]|#pt[나]"); }
}
