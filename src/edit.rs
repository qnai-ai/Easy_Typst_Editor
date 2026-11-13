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
pub enum Style { Bold, Under }

pub struct Work {
    pub text: String,
    pub caret: usize,
    pub anchor: usize,
}

impl Work {
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
}

impl Wrap {
    fn is_style(&self, st: Style) -> bool {
        match (&self.kind, st) {
            (WK::Star, Style::Bold) => true,
            (WK::Func(n, false), Style::Bold) => n == "strong",
            (WK::Func(n, false), Style::Under) => n == "underline" || n == "u",
            _ => false,
        }
    }
    fn inline(&self) -> bool {
        match &self.kind { WK::Func(n, _) => INLINE.contains(&n.as_str()), _ => true }
    }
    fn is_hang(&self, text: &str) -> bool {
        matches!(&self.kind, WK::Func(n, true) if n == "par") && text[self.open.clone()].contains("hanging-indent")
    }
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
            Some(Wrap {
                open: hash.range().start..cb.start + 1,
                close: cb.end - 1..cb.end,
                content: cb.start + 1..cb.end - 1,
                kind: WK::Func(callee.get().full_text().to_string(), ach.len() > 1),
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
        if w.inline() || w.is_hang(text) {
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
    let mut w2 = Work { text: w.text.clone(), caret: r.start, anchor: r.end };
    let (c0, a0) = (w.caret, w.anchor);
    let mut extra = Work { text: String::new(), caret: c0, anchor: a0 };
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
    if w.sel().is_some() { delete_sel(w)?; }
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
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

enum Tg { Del(Range<usize>, bool), Move(usize), Stop(Option<&'static str>) }

const CANT: &str = "이 요소는 지면에서 지울 수 없습니다.";

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

fn back_target(root: &LinkedNode, text: &str, p: usize) -> Tg {
    if let Some(sr) = str_at(root, p) {
        if p <= sr.start { return Tg::Stop(None); }
        let g = text[sr.start..p].graphemes(true).next_back().map(|g| g.len()).unwrap_or(0);
        return Tg::Del(p - g..p, true);
    }
    let Some(leaf) = root.leaf_at(p, Side::Before) else { return Tg::Stop(None) };
    let r = leaf.range();
    let in_markup = leaf.parent_kind() == Some(SyntaxKind::Markup);
    match leaf.kind() {
        SyntaxKind::Text if in_markup => {
            let g = text[r.start..p].graphemes(true).next_back().map(|g| g.len()).unwrap_or(0);
            Tg::Del(p - g..p, true)
        }
        SyntaxKind::Space if in_markup => {
            if leaf.index() == 0 { Tg::Stop(None) } else { Tg::Del(r.start..p, true) }
        }
        SyntaxKind::Parbreak if in_markup => {
            if leaf.index() == 0 { Tg::Stop(None) } else { Tg::Del(r.start..p, true) }
        }
        SyntaxKind::Escape | SyntaxKind::Shorthand | SyntaxKind::SmartQuote if in_markup => Tg::Del(r, true),
        SyntaxKind::Linebreak if in_markup => {
            let end = if text[p..].starts_with(' ') { p + 1 } else { p };
            Tg::Del(r.start..end, true)
        }
        SyntaxKind::Semicolon => Tg::Move(r.start),
        SyntaxKind::Star | SyntaxKind::Underscore => match leaf.parent().and_then(wrap_of) {
            Some(_) => Tg::Move(r.start),
            None => Tg::Stop(Some(CANT)),
        },
        SyntaxKind::RightBracket => match func_of(&leaf) {
            Some(f) => match wrap_of(&f) {
                Some(wp) if wp.inline() || wp.is_hang(text) => Tg::Move(r.start),
                _ => Tg::Stop(Some(CANT)),
            },
            None => Tg::Stop(Some(CANT)),
        },
        SyntaxKind::LeftBracket => match func_of(&leaf) {
            Some(f) => match wrap_of(&f) {
                Some(wp) if wp.inline() => Tg::Move(wp.open.start),
                _ => Tg::Stop(None),
            },
            None => Tg::Stop(None),
        },
        SyntaxKind::RightParen => match func_of(&leaf) {
            Some(f) if callee_name(&f) == "h" && f.range().end == p => match hash_start(&f) {
                Some(h) => Tg::Del(h..p, false),
                None => Tg::Stop(Some(CANT)),
            },
            _ => Tg::Stop(Some(CANT)),
        },
        _ => Tg::Stop(Some(CANT)),
    }
}

fn fwd_target(root: &LinkedNode, text: &str, p: usize) -> Tg {
    if let Some(sr) = str_at(root, p) {
        if p >= sr.end { return Tg::Stop(None); }
        let g = text[p..sr.end].graphemes(true).next().map(|g| g.len()).unwrap_or(0);
        return Tg::Del(p..p + g, true);
    }
    let Some(leaf) = root.leaf_at(p, Side::After) else { return Tg::Stop(None) };
    let r = leaf.range();
    let in_markup = leaf.parent_kind() == Some(SyntaxKind::Markup);
    let last = |l: &LinkedNode| l.parent().map(|p| p.children().len() == l.index() + 1).unwrap_or(false);
    match leaf.kind() {
        SyntaxKind::Text if in_markup => {
            let g = text[p..r.end].graphemes(true).next().map(|g| g.len()).unwrap_or(0);
            Tg::Del(p..p + g, true)
        }
        SyntaxKind::Space | SyntaxKind::Parbreak if in_markup => {
            if last(&leaf) { Tg::Stop(None) } else { Tg::Del(p..r.end, true) }
        }
        SyntaxKind::Escape | SyntaxKind::Shorthand | SyntaxKind::SmartQuote if in_markup => Tg::Del(r, true),
        SyntaxKind::Linebreak if in_markup => {
            let end = if text[r.end..].starts_with(' ') { r.end + 1 } else { r.end };
            Tg::Del(r.start..end, true)
        }
        SyntaxKind::Semicolon => Tg::Move(r.end),
        SyntaxKind::Star | SyntaxKind::Underscore => match leaf.parent().and_then(wrap_of) {
            Some(_) => Tg::Move(r.end),
            None => Tg::Stop(Some(CANT)),
        },
        SyntaxKind::Hash => match leaf.next_sibling() {
            Some(f) if f.kind() == SyntaxKind::FuncCall => match wrap_of(&f) {
                Some(wp) if wp.inline() || wp.is_hang(text) => Tg::Move(wp.open.end),
                _ if callee_name(&f) == "h" => {
                    let mut end = f.range().end;
                    if text[end..].starts_with(';') { end += 1; }
                    Tg::Del(r.start..end, false)
                }
                _ => Tg::Stop(Some(CANT)),
            },
            _ => Tg::Stop(Some(CANT)),
        },
        SyntaxKind::RightBracket => match func_of(&leaf) {
            Some(f) => match wrap_of(&f) {
                Some(wp) if wp.inline() || wp.is_hang(text) => Tg::Move(r.end),
                _ => Tg::Stop(None),
            },
            None => Tg::Stop(None),
        },
        _ => Tg::Stop(Some(CANT)),
    }
}

fn unchecked(w: &mut Work, r: Range<usize>, with: &str, after: bool) -> R<()> {
    let mut t = w.text.clone();
    t.replace_range(r.clone(), with);
    if bad(&tree(&t)) { return Err("이 자리는 지면에서 바로 고칠 수 없습니다.".into()); }
    w.rep(r, with, after);
    Ok(())
}

/// 내용이 비어 버린 꾸밈(`**`, `#u[]`)을 걷어 낸다.
fn cleanup_empty(w: &mut Work) {
    for _ in 0..6 {
        let node = tree(&w.text);
        let root = LinkedNode::new(&node);
        let p = w.caret;
        let Some(wp) = wraps_at(&root, p).into_iter().next() else { return };
        if !wp.content.is_empty() || !(wp.inline() || wp.is_hang(&w.text)) { return; }
        let mut end = wp.close.end;
        if matches!(wp.kind, WK::Func(..)) && w.text[end..].starts_with(';') { end += 1; }
        let mut t = w.text.clone();
        t.replace_range(wp.open.start..end, "");
        if bad(&tree(&t)) { return; }
        w.rep(wp.open.start..end, "", false);
        w.anchor = w.caret;
    }
}

fn del_one(w: &mut Work, fwd: bool) -> R<()> {
    let mut p = w.caret;
    for _ in 0..10 {
        let node = tree(&w.text);
        let root = LinkedNode::new(&node);
        let tg = if fwd { fwd_target(&root, &w.text, p) } else { back_target(&root, &w.text, p) };
        match tg {
            Tg::Move(q) => { if q == p { return Ok(()); } p = q; }
            Tg::Stop(None) => return Ok(()),
            Tg::Stop(Some(m)) => return Err(m.into()),
            Tg::Del(r, check) => {
                if r.is_empty() { return Ok(()); }
                w.caret = p; w.anchor = p;
                if check { checked(w, r, "")?; } else { unchecked(w, r, "", false)?; }
                w.anchor = w.caret;
                cleanup_empty(w);
                return Ok(());
            }
        }
    }
    Ok(())
}

pub fn backspace(w: &mut Work) -> R<()> { if w.sel().is_some() { delete_sel(w) } else { del_one(w, false) } }
pub fn delete(w: &mut Work) -> R<()> { if w.sel().is_some() { delete_sel(w) } else { del_one(w, true) } }

const CROSS: &str = "서로 다른 글 상자에 걸친 선택은 한꺼번에 고칠 수 없습니다. 한 문단(글 상자) 안에서 선택해 주세요.";

fn same_block(root: &LinkedNode, text: &str, a: usize, b: usize) -> R<()> {
    match (str_at(root, a), str_at(root, b)) {
        (Some(x), Some(y)) if x == y => return Ok(()),
        (None, None) => {}
        _ => return Err(CROSS.into()),
    }
    match (block_at(root, text, a), block_at(root, text, b)) {
        (Some(x), Some(y)) if x == y => Ok(()),
        _ => Err(CROSS.into()),
    }
}

/// 선택 구간에서 지울 조각을 모은다(꾸밈 표식은 남긴다).
fn collect_del(node: &LinkedNode, a: usize, b: usize, out: &mut Vec<Range<usize>>, sig: &mut Sig) {
    for c in node.children() {
        let r = c.range();
        if r.end <= a || r.start >= b { continue; }
        if a <= r.start && r.end <= b {
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

pub fn delete_sel(w: &mut Work) -> R<()> {
    if w.sel().is_none() { return Ok(()); }
    if delete_sel_once(w).is_ok() { return Ok(()); }
    // *굵게* 표식이 낱말에 붙어 깨지는 경우: 함수 꼴로 바꾼 뒤 다시
    let (a, b) = w.sel().unwrap();
    let node = tree(&w.text);
    let root = LinkedNode::new(&node);
    let mut ws = vec![];
    all_wraps(&root, &mut ws);
    ws.retain(|x| matches!(x.kind, WK::Star | WK::Under) && x.open.start <= b + 1 && x.close.end + 1 >= a);
    if ws.is_empty() { return delete_sel_once(w); }
    let saved = (w.text.clone(), w.caret, w.anchor);
    w.anchor = a; w.caret = b;
    ws.sort_by_key(|x| std::cmp::Reverse(x.open.start));
    for wp in &ws { to_func(w, wp); }
    let r = delete_sel_once(w);
    if r.is_err() { w.text = saved.0; w.caret = saved.1; w.anchor = saved.2; }
    r
}

fn delete_sel_once(w: &mut Work) -> R<()> {
    let Some((a, b)) = w.sel() else { return Ok(()) };
    let node = tree(&w.text);
    let root = LinkedNode::new(&node);
    same_block(&root, &w.text, a, b)?;
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
    cleanup_empty(w);
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
    let splittable = |wp: &Wrap| wp.inline() || wp.is_hang(&w.text);
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
    let m = block_at(&root, &w.text, p).ok_or("이 글에는 내어쓰기를 줄 수 없습니다.")?;
    let mnode = find_node(&root, &m, SyntaxKind::Markup).ok_or("이 글에는 내어쓰기를 줄 수 없습니다.")?;
    // 커서가 든 문단: 문단 나눔 사이
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
    let (Some(s), Some(e)) = (s, e) else { return Err("빈 문단에는 내어쓰기를 줄 수 없습니다.".into()) };
    let close = if needs_guard(next_char(&w.text, e)) { "];" } else { "]" };
    w.rep(e..e, close, false);
    w.rep(s..s, "#par(hanging-indent: 1em)[", true);
    if bad(&tree(&w.text)) { return Err("이 문단에는 내어쓰기를 줄 수 없습니다.".into()); }
    Ok(())
}

// ───────────────────────── 굵게·밑줄

fn text_leaves(node: &LinkedNode, a: usize, b: usize, out: &mut Vec<Range<usize>>) {
    for c in node.children() {
        let r = c.range();
        if r.end <= a || r.start >= b { continue; }
        if is_markup_text(&c) { out.push(r); } else { text_leaves(&c, a, b, out); }
    }
}

/// (굵게, 밑줄) 상태.
pub fn fmt_state(text: &str, a: usize, b: usize) -> (bool, bool) {
    let node = tree(text);
    let root = LinkedNode::new(&node);
    let has = |p: usize, st: Style| wraps_at(&root, p).iter().any(|w| w.is_style(st));
    if a == b { return (has(a, Style::Bold), has(a, Style::Under)); }
    let mut leaves = vec![];
    text_leaves(&root, a, b, &mut leaves);
    if leaves.is_empty() { return (false, false); }
    let all = |st: Style| leaves.iter().all(|r| has(r.start.max(a) + 1, st) || has(r.start.max(a), st) && has(r.end.min(b), st));
    (all(Style::Bold), all(Style::Under))
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
        let close_txt = if func { "]" } else { "*" };
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

fn add_style(w: &mut Work, st: Style, under_name: &str) -> R<()> {
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
            let star_ok = st == Style::Bold && pass == 0
                && !(wordy(prev_char(t, r.start)) && wordy(next_char(t, r.start)))
                && !(wordy(prev_char(t, r.end)) && wordy(next_char(t, r.end)))
                && prev_char(t, r.start) != Some('\\')
                && !t[r.clone()].contains('*');
            let (open, close) = if star_ok { ("*".to_string(), "*".to_string()) } else {
                let o = if st == Style::Bold { "#strong[" } else { under_name };
                (o.to_string(), if needs_guard(next_char(t, r.end)) { "];".to_string() } else { "]".to_string() })
            };
            w.rep(r.end..r.end, &close, false);
            w.rep(r.start..r.start, &open, true);
        }
        if !bad(&tree(&w.text)) { return Ok(()); }
        w.text = before.clone(); w.caret = c0; w.anchor = a0;
    }
    Err("이 선택에는 서식을 줄 수 없습니다.".into())
}

/// 굵게·밑줄을 켜거나 끈다. `under_fn`: 밑줄에 쓸 함수 이름(책의 템플릿에 `u`가 있으면 그것).
pub fn toggle(w: &mut Work, st: Style, under_fn: &str) -> R<()> {
    let Some((a, b)) = w.sel() else { return Err("먼저 서식을 바꿀 글자를 끌어서 선택해 주세요.".into()) };
    let node = tree(&w.text);
    let root = LinkedNode::new(&node);
    if str_at(&root, a).is_some() || str_at(&root, b).is_some() {
        return Err("제목처럼 따옴표 안에 든 글에는 굵게·밑줄을 줄 수 없습니다.".into());
    }
    same_block(&root, &w.text, a, b)?;
    let (bold, under) = fmt_state(&w.text, a, b);
    let on = if st == Style::Bold { bold } else { under };
    // 표시를 선택 범위로 맞춘다(시작 = anchor, 끝 = caret)
    w.anchor = a; w.caret = b;
    remove_style(w, st)?;
    if !on { add_style(w, st, &format!("#{under_fn}["))?; }
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
        // 마크업: 앞뒤가 문단 나눔이어야 한다
        let prev = f.prev_sibling_with_trivia().and_then(|h| h.prev_sibling_with_trivia());
        let prev = match prev { Some(p) if p.kind() == SyntaxKind::Space => p.prev_sibling_with_trivia(), o => o };
        let next = match f.next_sibling_with_trivia() { Some(n) if n.kind() == SyntaxKind::Space => n.next_sibling_with_trivia(), o => o };
        return prev.map_or(true, |p| p.kind() == SyntaxKind::Parbreak) && next.map_or(true, |n| n.kind() == SyntaxKind::Parbreak);
    }
    match f.parent_kind() {
        Some(SyntaxKind::Code) => f.prev_sibling().map_or(true, |p| blocky(&p)) && f.next_sibling().map_or(true, |n| blocky(&n)),
        Some(SyntaxKind::Args) => f.parent().and_then(|a| a.parent().cloned()).is_some_and(|c| BLOCKY.contains(&callee_name(&c).as_str())),
        _ => false,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    fn w(text: &str) -> Work {
        // `|` 커서, `<`..`>` 대신 `{`,`}` 로 선택
        if let (Some(a), Some(b)) = (text.find('{'), text.find('}')) {
            let t = text.replace(['{', '}'], "");
            Work { text: t, anchor: a, caret: b - 1 }
        } else {
            let p = text.find('|').unwrap();
            Work { text: text.replace('|', ""), caret: p, anchor: p }
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
    #[test] fn del_sel_cross() { let mut x = w("#ex[a{b][c}d]"); assert!(delete_sel(&mut x).is_err()); }
}
