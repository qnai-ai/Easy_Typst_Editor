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
                if c.kind() == SyntaxKind::Text { return Some(r); }
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
                kind: WK::Func(name, ach.len() > 1),
            })
        }
        _ => None,
    }
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
                }
                if let Some(wp) = wrap_of(&f) {
                    if wp.content.start == r.end && wp.flows(text) { return Tg::Move(wp.open.start); }
                }
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
                }
                if let Some(wp) = wrap_of(&f) {
                    if wp.close == r && wp.flows(text) { return Tg::Move(r.end); }
                }
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
                w.caret = r.start; w.anchor = r.start;
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

/// 글자 꾸밈 상태: [굵게, 기울임, 밑줄, 취소선].
pub fn fmt_state(text: &str, a: usize, b: usize) -> [bool; 4] {
    let node = tree(text);
    let root = LinkedNode::new(&node);
    let has = |p: usize, st: Style| wraps_at(&root, p).iter().any(|w| w.is_style(st));
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

fn remove_style(w: &mut Work, st: Style) -> R<()> {
    for _ in 0..200 {
        let (a, b) = match w.sel() { Some(x) => x, None => return Ok(()) };
        let node = tree(&w.text);
        let root = LinkedNode::new(&node);
        let mut ws = vec![];
        all_wraps(&root, &mut ws);
        let Some(wp) = ws.into_iter().find(|x| x.is_style(st) && a.max(x.content.start) < b.min(x.content.end)) else { return Ok(()) };
        let (cs, ce) = (wp.content.start, wp.content.end);
        let os = snap(&root, &wp.content, a.max(cs), false);
        let oe = snap(&root, &wp.content, b.min(ce), true);
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
            // 굵게·기울임은 되도록 짧은 표식(`*`, `_`)으로, 낱말 한가운데 등 표식이 깨지는 자리에서는 함수 꼴로
            let mark = match st { Style::Bold => Some("*"), Style::Italic => Some("_"), _ => None };
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

/// 굵게·기울임·밑줄·취소선을 켜거나 끈다. `under_fn`: 밑줄에 쓸 함수 이름(책의 템플릿에 `u`가 있으면 그것).
/// 여러 글 상자에 걸친 선택이면 상자마다 따로 꾸민다.
pub fn toggle(w: &mut Work, st: Style, under_fn: &str) -> R<()> {
    let Some((a, b)) = w.sel() else { return Err("먼저 서식을 바꿀 글자를 끌어서 선택해 주세요.".into()) };
    let node = tree(&w.text);
    let root = LinkedNode::new(&node);
    if str_at(&root, a).is_some() || str_at(&root, b).is_some() {
        return Err("제목처럼 따옴표 안에 든 글에는 굵게·밑줄 같은 꾸밈을 줄 수 없습니다.".into());
    }
    let on = fmt_state(&w.text, a, b)[STYLES.iter().position(|x| *x == st).unwrap()];
    // 표시를 선택 범위로 맞춘다(시작 = anchor, 끝 = caret)
    w.anchor = a; w.caret = b;
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

/// 상자·표의 너비를 `dx`pt 만큼 바꾼다.
/// `col`: None이면 오른쪽 바깥 가장자리, Some(i)면 i번째 칸과 그다음 칸 사이의 경계.
/// `width`: 지금 지면에서의 전체 너비, `cols`: 지금 칸마다의 너비(표·격자).
pub fn elem_resize(text: &str, r: &Range<usize>, col: Option<usize>, dx: f32, width: Option<f32>, cols: &[f32]) -> R<String> {
    let node = tree(text);
    let root = LinkedNode::new(&node);
    let f = find_node(&root, r, SyntaxKind::FuncCall).ok_or(NO_ELEM)?;
    let name = callee_name(&f);
    let args = f.children().find(|c| c.kind() == SyntaxKind::Args).ok_or(NO_ELEM)?;
    let named = |key: &str| args.children().find(|c| c.kind() == SyntaxKind::Named
        && c.children().next().is_some_and(|k| k.get().full_text() == key));
    let mut t = text.to_string();
    if name == "grid" || name == "table" {
        let val = named("columns").and_then(|n| n.children().last()).ok_or(NO_COLS)?;
        // 칸마다의 표기와 원고 자리(배열이 아니면 값 전체를 새 배열로 바꿔 쓴다)
        let mut items: Vec<(Option<Range<usize>>, String)> = vec![];
        match val.kind() {
            SyntaxKind::Array => for c in val.children() {
                match c.kind() {
                    SyntaxKind::LeftParen | SyntaxKind::RightParen | SyntaxKind::Comma | SyntaxKind::Space
                    | SyntaxKind::LineComment | SyntaxKind::BlockComment => {}
                    SyntaxKind::Spread | SyntaxKind::Named | SyntaxKind::Keyed => return Err(NO_COLS.into()),
                    _ => items.push((Some(c.range()), text[c.range()].to_string())),
                }
            },
            SyntaxKind::Int => {
                let n: usize = text[val.range()].trim().parse().map_err(|_| NO_COLS.to_string())?;
                items = vec![(None, "auto".to_string()); n];
            }
            _ => items.push((None, text[val.range()].to_string())),
        }
        let n = items.len();
        if n == 0 || items.iter().any(|x| matches!(parse_len(&x.1), Len::Other)) { return Err(NO_COLS.into()); }
        let w = |i: usize| cols.get(i).copied().filter(|x| x.is_finite() && *x > 0.0 && cols.len() == n);
        let is_fr = |i: usize| matches!(parse_len(&items[i].1), Len::Fr(_));
        let mut set: Vec<(usize, String)> = vec![];
        match col {
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
            t.replace_range(val.range(), &format!("({}{})", list.join(", "), if n == 1 { "," } else { "" }));
        }
    } else if SIZED.contains(&name.as_str()) {
        let d = match width { Some(c) => dx.max(MIN_W - c), None => dx };
        match named("width").and_then(|n| n.children().last()) {
            Some(val) => { let v = set_len(&text[val.range()], width, d)?; t.replace_range(val.range(), &v); }
            None => {
                let v = abs_mm(width.ok_or(NO_ELEM)? + d);
                match args.children().next() {
                    Some(lp) if lp.kind() == SyntaxKind::LeftParen => {
                        let empty = args.children().nth(1).is_some_and(|c| c.kind() == SyntaxKind::RightParen);
                        t.insert_str(lp.range().end, &if empty { format!("width: {v}") } else { format!("width: {v}, ") });
                    }
                    _ => t.insert_str(args.range().start, &format!("(width: {v})")),
                }
            }
        }
    } else {
        return Err("이 요소는 지면에서 너비를 바꿀 수 없습니다.".into());
    }
    if bad(&tree(&t)) { return Err("이 요소는 지면에서 너비를 바꿀 수 없습니다.".into()); }
    Ok(t)
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
    #[test] fn bold_part_off() { let mut x = w("*{big} cat*"); toggle(&mut x, Style::Bold, "underline").unwrap(); assert_eq!(x.text, "big* cat*"); }
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
    #[test] fn bs_dead_str() {
        let mut x = w("가\n\n#sub(\"기|\")\n\n나");
        backspace(&mut x).unwrap(); assert_eq!(show(&x), "가\n\n#sub(\"|\")\n\n나");
        backspace(&mut x).unwrap(); assert_eq!(show(&x), "가\n\n|\n나"); assert_eq!(x.note.as_deref(), Some("sub"));
        let mut y = w("#unit(title: \"|\", root: \"audi\")"); backspace(&mut y).unwrap(); assert!(y.edge);
    }
    // 상자·표의 너비
    fn call(t: &str, name: &str) -> Range<usize> { let s = t.find(name).unwrap(); let node = tree(t); let root = LinkedNode::new(&node); let mut n = root.leaf_at(s + 1, Side::Before).unwrap(); while n.kind() != SyntaxKind::FuncCall { n = n.parent().unwrap().clone(); } n.range() }
    const MM: f32 = 2.834_645_7;
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
