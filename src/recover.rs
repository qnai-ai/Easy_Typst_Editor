//! 원본 위치 복원.
//!
//! 템플릿이 `show regex(..)` 등으로 글자를 다시 만들어 내면 Typst는 그 글자가 원본 어디에서
//! 왔는지 잊는다(한글 본문이 흔히 그렇다). 여기서는 "지면에 찍힌 글"과 "원고의 글"을 글자로
//! 대조해 그 자리를 되찾는다. 글이 정확히 같을 때에만 잇는다.

use std::collections::HashMap;

use typst::syntax::{FileId, LinkedNode, SyntaxKind};

use crate::index::{Index, K_ATOM, K_LOCK, K_NONE, K_STR, K_TEXT};
use crate::world::ProjectWorld;

const SENT: char = '\0';

struct Sc { ch: char, gi: u32 }

struct Tok {
    file: FileId,
    kind: u8,
    /// 공백을 뺀 글자와 그 글자의 원본 바이트 위치
    norm: Vec<(char, u32)>,
    /// 앞뒤로 이미 자리를 아는 글자가 붙어 있으면 그 경계 바이트
    s: u32,
    w0: u32,
    w1: u32,
    /// 넓히지 않은 찾는 범위(쪽)
    n0: u32,
    n1: u32,
    done: bool,
}

pub fn recover(world: &ProjectWorld, ix: &mut Index, changed: &[bool]) {
    if ix.glyphs.is_empty() || !changed.iter().any(|c| *c) { return; }
    if verify_native(world, ix, changed) { ix.index_files(); }
    // 1. 자리를 모르는 글자들의 흐름
    let mut stream: Vec<Sc> = Vec::with_capacity(ix.glyphs.len());
    let mut spos: Vec<u32> = Vec::with_capacity(ix.glyphs.len());
    let npages = ix.page_ranges.len();
    let mut page_start: Vec<u32> = vec![0; npages + 1];
    for (p, &(a, b)) in ix.page_ranges.iter().enumerate() {
        page_start[p] = stream.len() as u32;
        if !changed[p] { for _ in a..b { spos.push(stream.len() as u32); } continue; }
        for gi in a..b {
            spos.push(stream.len() as u32);
            let g = &ix.glyphs[gi];
            if g.kind == K_TEXT || g.kind == K_ATOM {
                stream.push(Sc { ch: SENT, gi: gi as u32 });
            } else {
                let t = ix.text_of(gi);
                for ch in t.chars() {
                    if ch.is_whitespace() { continue; }
                    stream.push(Sc { ch, gi: gi as u32 });
                }
            }
        }
    }
    page_start[npages] = stream.len() as u32;
    let mut used = vec![false; stream.len()];
    let mut pair: HashMap<(char, char), Vec<u32>> = HashMap::new();
    let mut single: HashMap<char, Vec<u32>> = HashMap::new();
    for i in 0..stream.len() {
        if stream[i].ch == SENT { continue; }
        single.entry(stream[i].ch).or_default().push(i as u32);
        if i + 1 < stream.len() && stream[i + 1].ch != SENT {
            pair.entry((stream[i].ch, stream[i + 1].ch)).or_default().push(i as u32);
        }
    }

    // 2. 원고에서 아직 지면과 이어지지 않은 글 조각
    let mut toks: Vec<Tok> = vec![];
    let ids: Vec<FileId> = {
        let files = world.files.lock().unwrap();
        files.iter().filter_map(|(id, e)| match e { Ok(e) if e.in_project && e.source.is_some() => Some(*id), _ => None }).collect()
    };
    for id in ids {
        let Some(src) = world.source_of(id) else { continue };
        // 이미 이어진 구간
        let mut cov: Vec<(u32, u32)> = vec![];
        let mut anchors: Vec<(u32, u32)> = vec![]; // (원본 위치, 쪽)
        if let Some(v) = ix.by_file.get(&id) {
            for &(s, gi) in v {
                let g = &ix.glyphs[gi as usize];
                anchors.push((s, g.page));
                if g.kind == K_TEXT || g.kind == K_STR { cov.push((g.s, g.e)); }
            }
        }
        cov.sort();
        let mut merged: Vec<(u32, u32)> = vec![];
        for (s, e) in cov {
            match merged.last_mut() { Some(l) if s <= l.1 => l.1 = l.1.max(e), _ => merged.push((s, e)) }
        }
        let text = src.text();
        let root = LinkedNode::new(src.root());
        let before = toks.len();
        collect(&root, text, id, &merged, false, &mut toks);
        for t in &mut toks[before..] {
            let i = anchors.partition_point(|a| a.0 <= t.s);
            let pa = if i > 0 { Some(anchors[i - 1].1) } else { None };
            let pb = anchors.get(i).map(|a| a.1);
            let (a, b) = match (pa, pb) {
                (Some(a), Some(b)) => (a.min(b), a.max(b)),
                (Some(a), None) | (None, Some(a)) => (a, a),
                (None, None) => (0, npages as u32 - 1),
            };
            t.n0 = a; t.n1 = b;
            t.w0 = a.saturating_sub(1);
            t.w1 = (b + 1).min(npages as u32 - 1);
        }
    }

    // 3. 같은 글끼리 묶어, 긴 것부터 지면에서 찾는다
    let last_page = npages as u32 - 1;
    for _round in 0..2 {
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (ti, t) in toks.iter().enumerate() {
        if !t.done { groups.entry(t.norm.iter().map(|c| c.0).collect()).or_default().push(ti); }
    }
    let mut keys: Vec<(usize, String)> = groups.keys().map(|k| (k.chars().count(), k.clone())).collect();
    keys.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    for (len, key) in keys {
        let mut members = groups.remove(&key).unwrap();
        let norm: Vec<(char, u32)> = toks[members[0]].norm.clone();
        let cands: &[u32] = if len >= 2 {
            pair.get(&(norm[0].0, norm[1].0)).map(|v| v.as_slice()).unwrap_or(&[])
        } else {
            single.get(&norm[0].0).map(|v| v.as_slice()).unwrap_or(&[])
        };
        let find = |used: &[bool], w0: u32, w1: u32| -> Vec<usize> {
            let (lo, hi) = (page_start[w0 as usize], page_start[w1 as usize + 1]);
            let a = cands.partition_point(|&p| p < lo);
            let mut occ = vec![];
            let mut last_end = 0usize;
            for &p in &cands[a..] {
                if p >= hi { break; }
                let p = p as usize;
                if p < last_end { continue; }
                if matches(&stream, used, p, &norm) { occ.push(p); last_end = p + len; }
            }
            occ
        };
        // 3-1. 앞뒤 글자가 원고에서도 바로 이웃인 자리(문맥이 맞는 자리)에 잇는다
        for &ti in &members {
            let occ = find(&used, toks[ti].w0, toks[ti].w1);
            for at in occ {
                if fits(ix, &stream, &toks[ti], at) && context_ok(ix, &stream, &toks[ti], at, len) {
                    apply(ix, &stream, &mut used, &toks[ti], at);
                    toks[ti].done = true;
                }
            }
            // 찾는 범위가 빗나갔을 수 있다(가까이에 자리를 아는 글자가 없는 템플릿 글 등): 책 전체에서 문맥으로 찾는다
            if !toks[ti].done && len >= 2 && cands.len() <= 3000 {
                for at in find(&used, 0, last_page) {
                    if fits(ix, &stream, &toks[ti], at) && context_ok(ix, &stream, &toks[ti], at, len) {
                        apply(ix, &stream, &mut used, &toks[ti], at);
                        toks[ti].done = true;
                    }
                }
            }
        }
        members.retain(|&ti| !toks[ti].done);
        // 3-2. 남은 것은 원고의 개수와 지면의 개수가 꼭 같을 때에만 차례대로 잇는다(좁은 범위 먼저)
        for wide in [false, true] {
            members.retain(|&ti| !toks[ti].done);
            let win = |t: &Tok| if wide { (t.w0, t.w1) } else { (t.n0, t.n1) };
            members.sort_by_key(|&ti| (win(&toks[ti]).0, toks[ti].s));
            let mut i = 0;
            while i < members.len() {
                let mut j = i + 1;
                let (w0, mut w1) = win(&toks[members[i]]);
                while j < members.len() && win(&toks[members[j]]).0 <= w1 { w1 = w1.max(win(&toks[members[j]]).1); j += 1; }
                let mut occ = find(&used, w0, w1);
                if members[i..j].iter().any(|&ti| toks[ti].kind == K_STR) { occ.retain(|&at| fits(ix, &stream, &toks[members[i]], at) || toks[members[i]].kind != K_STR && uniform(ix, &stream, at, len)); occ.retain(|&at| uniform(ix, &stream, at, len)); }
                let k = j - i;
                if occ.len() == k && (len >= 2 || k == 1) {
                    for (m, &ti) in members[i..j].iter().enumerate() {
                        apply(ix, &stream, &mut used, &toks[ti], occ[m]);
                        toks[ti].done = true;
                    }
                }
                i = j;
            }
        }
        // 3-3. 책 전체에서 원고에도 지면에도 꼭 하나뿐인 글
        members.retain(|&ti| !toks[ti].done);
        if members.len() == 1 && len >= 4 {
            let occ = find(&used, 0, last_page);
            if occ.len() == 1 && fits(ix, &stream, &toks[members[0]], occ[0]) {
                apply(ix, &stream, &mut used, &toks[members[0]], occ[0]);
                toks[members[0]].done = true;
            }
        }
    }
    }

    // 낱말 사이 공백
    let mut texts: HashMap<FileId, typst::syntax::Source> = HashMap::new();
    for gi in 1..ix.glyphs.len().saturating_sub(1) {
        let g = &ix.glyphs[gi];
        if g.kind != K_NONE && g.kind != K_LOCK { continue; }
        if !changed[g.page as usize] { continue; }
        if !ix.text_of(gi).chars().all(|c| c.is_whitespace()) || ix.text_of(gi).is_empty() { continue; }
        let (p, n) = (&ix.glyphs[gi - 1], &ix.glyphs[gi + 1]);
        let editable = |k: u8| k == K_TEXT || k == K_STR || k == K_ATOM;
        if !editable(p.kind) || !editable(n.kind) || p.file != n.file || p.e >= n.s { continue; }
        let Some(f) = p.file else { continue };
        let src = texts.entry(f).or_insert_with(|| world.source_of(f).unwrap());
        let Some(between) = src.text().get(p.e as usize..n.s as usize) else { continue };
        if between.is_empty() || !between.chars().all(|c| c == ' ') { continue; }
        let kind = if p.kind == K_STR && n.kind == K_STR { K_STR } else { K_ATOM };
        let (s, e) = (p.e, n.s);
        let g = &mut ix.glyphs[gi];
        g.file = Some(f); g.s = s; g.e = e; g.kind = kind; g.node_kind = SyntaxKind::Space; g.rec = true;
    }
}

fn matches(stream: &[Sc], used: &[bool], at: usize, norm: &[(char, u32)]) -> bool {
    if at + norm.len() > stream.len() { return false; }
    for (j, c) in norm.iter().enumerate() {
        if used[at + j] || stream[at + j].ch != c.0 { return false; }
    }
    true
}

fn mark(used: &mut [bool], at: usize, n: usize) { for u in &mut used[at..at + n] { *u = true; } }

/// 조각을 지면의 그 자리에 잇는다: 글리프마다 원본 위치를 적는다.
fn apply(ix: &mut Index, stream: &[Sc], used: &mut [bool], t: &Tok, at: usize) {
    mark(used, at, t.norm.len());
    let mut j = 0;
    while j < t.norm.len() {
        let gi = stream[at + j].gi as usize;
        let mut k = j;
        while k + 1 < t.norm.len() && stream[at + k + 1].gi as usize == gi { k += 1; }
        let s = t.norm[j].1;
        let e = t.norm[k].1 + t.norm[k].0.len_utf8() as u32;
        let g = &mut ix.glyphs[gi];
        g.file = Some(t.file); g.s = s; g.e = e; g.kind = t.kind; g.rec = true;
        g.node_kind = if t.kind == K_STR { SyntaxKind::Str } else { SyntaxKind::Text };
        j = k + 1;
    }
}

/// 따옴표 문자열은 한 덩어리로 찍힌다고 본다: 글자 크기가 한결같아야 한다
/// (크기가 다른 두 줄을 이어 붙인 것이 우연히 같은 글이 되는 일을 막는다).
fn uniform(ix: &Index, stream: &[Sc], at: usize, len: usize) -> bool {
    let size = ix.glyphs[stream[at].gi as usize].size;
    (1..len).all(|j| (ix.glyphs[stream[at + j].gi as usize].size - size).abs() < 0.01)
}
fn fits(ix: &Index, stream: &[Sc], t: &Tok, at: usize) -> bool {
    t.kind != K_STR || uniform(ix, stream, at, t.norm.len())
}

/// 지면에서 이 자리의 앞뒤 글자가, 원고에서도 이 조각의 바로 앞뒤에 있는지.
fn context_ok(ix: &Index, stream: &[Sc], t: &Tok, at: usize, len: usize) -> bool {
    let first = stream[at].gi as usize;
    let last = stream[at + len - 1].gi as usize;
    let page = ix.glyphs[first].page;
    let known = |g: &crate::index::G| (g.kind == K_TEXT || g.kind == K_ATOM) && g.file == Some(t.file) && g.e > g.s;
    let te = t.norm[len - 1].1 + t.norm[len - 1].0.len_utf8() as u32;
    let mut dl: Option<i64> = None;
    let mut i = first;
    for _ in 0..40 {
        if i == 0 { break; }
        i -= 1;
        let g = &ix.glyphs[i];
        if g.page != page { break; }
        if g.kind == K_TEXT || g.kind == K_ATOM { if known(g) { dl = Some(t.s as i64 - g.e as i64); } break; }
    }
    let mut dr: Option<i64> = None;
    let mut i = last;
    for _ in 0..40 {
        i += 1;
        if i >= ix.glyphs.len() { break; }
        let g = &ix.glyphs[i];
        if g.page != page { break; }
        if g.kind == K_TEXT || g.kind == K_ATOM { if known(g) { dr = Some(g.s as i64 - te as i64); } break; }
    }
    let near = |d: Option<i64>, lim: i64| d.is_some_and(|d| d >= 0 && d <= lim);
    near(dl, 16) || near(dr, 16) || (near(dl, 400) && near(dr, 400))
}

fn collect(node: &LinkedNode, text: &str, file: FileId, cov: &[(u32, u32)], in_import: bool, out: &mut Vec<Tok>) {
    let kind = node.kind();
    match kind {
        SyntaxKind::Text if node.parent_kind() != Some(SyntaxKind::Raw) => {
            let r = node.range();
            let (ns, ne) = (r.start as u32, r.end as u32);
            // 덮이지 않은 구간들
            let mut cur = ns;
            let a = cov.partition_point(|c| c.1 <= ns);
            let mut gaps: Vec<(u32, u32, bool, bool)> = vec![];
            for c in &cov[a..] {
                if c.0 >= ne { break; }
                if c.0 > cur { gaps.push((cur, c.0, cur > ns, true)); }
                cur = cur.max(c.1);
            }
            if cur < ne { gaps.push((cur, ne, cur > ns, false)); }
            for (gs, ge, l, r) in gaps {
                let _ = (l, r);
                push_tok(text, file, gs, ge, K_TEXT, out);
            }
        }
        SyntaxKind::Str if !in_import => {
            let r = node.range();
            let a = cov.partition_point(|c| c.1 <= r.start as u32);
            let covered = cov.get(a).is_some_and(|c| c.0 < r.end as u32);
            if !covered && r.len() >= 3 && !text[r.clone()].contains('\\') {
                push_tok(text, file, r.start as u32 + 1, r.end as u32 - 1, K_STR, out);
            }
        }
        _ => {
            let imp = in_import || matches!(kind, SyntaxKind::ModuleImport | SyntaxKind::ModuleInclude);
            for c in node.children() { collect(&c, text, file, cov, imp, out); }
        }
    }
}

fn push_tok(text: &str, file: FileId, s: u32, e: u32, kind: u8, out: &mut Vec<Tok>) {
    let Some(t) = text.get(s as usize..e as usize) else { return };
    let norm: Vec<(char, u32)> = t.char_indices().filter(|c| !c.1.is_whitespace()).map(|(o, c)| (c, s + o as u32)).collect();
    if norm.is_empty() { return; }
    out.push(Tok { file, kind, norm, s, w0: 0, w1: 0, n0: 0, n1: 0, done: false });
}


/// Typst가 알려 준 글자 위치가 원고의 글과 정말 맞는지 확인한다.
/// 한 노드 안에서 하나라도 어긋나거나 빠지면(show 규칙이 글을 쪼갠 경우) 그 노드의 위치는 믿지 않는다.
fn verify_native(world: &ProjectWorld, ix: &mut Index, changed: &[bool]) -> bool {
    let mut nodes: HashMap<(FileId, u32, u32), Vec<u32>> = HashMap::new();
    for g in ix.glyphs.iter() {
        if g.kind == K_TEXT && !g.rec && changed[g.page as usize] {
            if let Some(f) = g.file { nodes.entry((f, g.node.0, g.node.1)).or_default(); }
        }
    }
    for (gi, g) in ix.glyphs.iter().enumerate() {
        if g.kind == K_TEXT && !g.rec {
            if let Some(f) = g.file {
                if let Some(v) = nodes.get_mut(&(f, g.node.0, g.node.1)) { v.push(gi as u32); }
            }
        }
    }
    let mut srcs: HashMap<FileId, typst::syntax::Source> = HashMap::new();
    let mut bad: Vec<u32> = vec![];
    for ((f, ns, ne), gis) in nodes {
        let src = srcs.entry(f).or_insert_with(|| world.source_of(f).unwrap());
        let Some(text) = src.text().get(ns as usize..ne as usize) else { bad.extend(gis); continue };
        let mut covered = vec![false; text.len()];
        let mut ok = true;
        for &gi in &gis {
            let g = &ix.glyphs[gi as usize];
            if g.s < ns || g.e > ne || g.e < g.s { ok = false; break; }
            let (a, b) = ((g.s - ns) as usize, (g.e - ns) as usize);
            let gt = ix.text_of(gi as usize);
            match text.get(a..b) {
                Some(t) if t == gt || (t.chars().all(|c| c.is_whitespace()) && gt.chars().all(|c| c.is_whitespace())) => {
                    for c in &mut covered[a..b] { *c = true; }
                }
                _ => { ok = false; break; }
            }
        }
        if ok {
            for (o, ch) in text.char_indices() {
                if !ch.is_whitespace() && !covered[o] { ok = false; break; }
            }
        }
        if !ok { bad.extend(gis); }
    }
    let mut any = false;
    for gi in bad {
        let g = &mut ix.glyphs[gi as usize];
        if !changed[g.page as usize] { continue; }
        g.kind = K_NONE; g.file = None;
        any = true;
    }
    any
}
