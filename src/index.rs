//! 지면 색인: 조판된 모든 글자(글리프)가 지면의 어디에 있고, 원본의 어느 바이트에서 왔는지 기록한다.

use std::collections::HashMap;

use typst::layout::{Frame, FrameItem, Point, Transform};
use typst::syntax::{FileId, Span, SyntaxKind};
use typst::visualize::Geometry;
use typst::World;
use typst_layout::PagedDocument;

use crate::world::ProjectWorld;

/// 글자 종류.
pub const K_TEXT: u8 = 0; // 마크업 본문 글자: 글자 단위로 편집
pub const K_ATOM: u8 = 1; // 공백·이스케이프·줄임표 등: 통째로 다루는 한 덩어리
pub const K_LOCK: u8 = 2; // 원본 위치는 알지만 지면에서 고칠 수 없음(코드·문자열·수식 등)
pub const K_NONE: u8 = 3; // 원본 위치를 알 수 없음(자동 생성)
pub const K_STR: u8 = 4; // 따옴표 문자열 안의 글자(제목 등): 글자만 고칠 수 있음
pub const K_MATH: u8 = 5; // 수식($..$) 안의 글자: 수식 원문을 통째로 고친다(s..e = 수식 전체)

#[derive(Clone, Debug)]
pub struct G {
    pub page: u32,
    pub x0: f32,
    pub x1: f32,
    /// 기준선 y
    pub y: f32,
    /// 글자 크기(pt)
    pub size: f32,
    pub line: u32,
    pub file: Option<FileId>,
    pub s: u32,
    pub e: u32,
    pub kind: u8,
    pub node_kind: SyntaxKind,
    pub item: u32,
    pub tr: (u16, u16),
    /// 글자가 속한 원본 노드의 범위(Typst가 알려 준 경우)
    pub node: (u32, u32),
    /// 본문 대조로 되찾은 자리인지
    pub rec: bool,
    /// 지면에 찍힌 굵기: 0 보통, 1 굵게, 2 알 수 없음(그 글꼴에 굵은 꼴이 없다)
    pub bold: u8,
    /// 지면에 그어진 글자 꾸밈 선: 1 밑줄, 2 취소선
    pub deco: u8,
    /// 지면에 찍힌 글자 색(RGB)
    pub fill: [u8; 3],
}

#[derive(Default)]
pub struct Index {
    pub glyphs: Vec<G>,
    /// 쪽마다 glyphs 안의 범위
    pub page_ranges: Vec<(usize, usize)>,
    /// 파일마다 (원본 시작 바이트, 글리프 번호) — 시작 바이트 순
    pub by_file: HashMap<FileId, Vec<(u32, u32)>>,
    pub item_text: Vec<typst::ecow::EcoString>,
    /// 쪽마다 item_text 안의 범위
    pub page_items: Vec<(usize, usize)>,
}

/// 앞선 색인을 다시 쓰기 위한 정보: 쪽 그림이 그대로인 쪽은 글자 자리만 밀어서 재사용한다.
pub struct Reuse<'a> {
    pub old: &'a Index,
    pub old_hashes: &'a [String],
    pub new_hashes: &'a [String],
    /// (파일, 고친 자리의 옛 끝 바이트, 늘어난 바이트 수)
    pub shifts: Vec<(FileId, u32, i64)>,
}

struct Builder<'a> {
    world: &'a ProjectWorld,
    cache: HashMap<Span, (Option<FileId>, u32, u32, u8, SyntaxKind)>,
    out: Index,
    line: u32,
    page: u32,
    /// 글꼴 가족마다 굵은 꼴이 있는지
    can_bold: HashMap<String, bool>,
}

impl Index {
    pub fn build(world: &ProjectWorld, doc: &PagedDocument, reuse: Option<Reuse>) -> Index {
        let mut b = Builder { world, cache: HashMap::new(), out: Index::default(), line: 0, page: 0, can_bold: HashMap::new() };
        let mut changed = vec![true; doc.pages().len()];
        for (i, page) in doc.pages().iter().enumerate() {
            b.page = i as u32;
            let start = b.out.glyphs.len();
            let istart = b.out.item_text.len();
            let same = reuse.as_ref().is_some_and(|r| {
                i < r.old.page_ranges.len() && i < r.old_hashes.len() && r.old_hashes[i] == r.new_hashes[i]
            });
            if same {
                let r = reuse.as_ref().unwrap();
                let (ga, gb) = r.old.page_ranges[i];
                let (ia, ib) = r.old.page_items[i];
                b.out.item_text.extend_from_slice(&r.old.item_text[ia..ib]);
                let (mut lmin, mut lmax) = (u32::MAX, 0u32);
                for g in &r.old.glyphs[ga..gb] { lmin = lmin.min(g.line); lmax = lmax.max(g.line); }
                let base = b.line + 1;
                for g in &r.old.glyphs[ga..gb] {
                    let mut g = g.clone();
                    g.item = g.item - ia as u32 + istart as u32;
                    g.line = g.line - lmin + base;
                    if let Some(f) = g.file {
                        for &(sf, old_end, delta) in &r.shifts {
                            if sf != f { continue; }
                            if g.s >= old_end {
                                g.s = (g.s as i64 + delta) as u32;
                                g.e = (g.e as i64 + delta) as u32;
                            }
                            if g.node.0 >= old_end { g.node.0 = (g.node.0 as i64 + delta) as u32; }
                            if g.node.1 >= old_end && g.node.1 > 0 { g.node.1 = (g.node.1 as i64 + delta) as u32; }
                        }
                    }
                    b.out.glyphs.push(g);
                }
                if ga < gb { b.line = base + (lmax - lmin); }
                changed[i] = false;
            } else {
                b.frame(&page.frame, Transform::identity());
            }
            b.out.page_ranges.push((start, b.out.glyphs.len()));
            b.out.page_items.push((istart, b.out.item_text.len()));
        }
        let mut out = b.out;
        out.adopt_math();
        out.index_files();
        crate::recover::recover(world, &mut out, &changed);
        out.index_files();
        out
    }

    /// 수식 안에서 표준 라이브러리가 만든 글자(`dif`의 d 등)는 원고 자리가 없다:
    /// 같은 줄에 바싹 붙은 수식 글자가 있으면 그 수식의 글자로 본다.
    fn adopt_math(&mut self) {
        let n = self.glyphs.len();
        for pass in 0..2 {
            for k in 0..n {
                let i = if pass == 0 { k } else { n - 1 - k };
                if self.glyphs[i].kind != K_NONE { continue; }
                let g = &self.glyphs[i];
                let near = |j: usize| {
                    let h = &self.glyphs[j];
                    h.kind == K_MATH && h.page == g.page && h.line == g.line
                        && (h.x0.min(h.x1) - g.x1.max(g.x0)).max(g.x0.min(g.x1) - h.x1.max(h.x0)) < g.size * 0.6
                };
                let from = if pass == 0 { i.checked_sub(1) } else { (i + 1 < n).then_some(i + 1) };
                if let Some(j) = from.filter(|&j| near(j)) {
                    let h = self.glyphs[j].clone();
                    let g = &mut self.glyphs[i];
                    g.kind = K_MATH; g.file = h.file; g.s = h.s; g.e = h.e; g.node = h.node; g.node_kind = h.node_kind;
                }
            }
        }
    }

    pub fn index_files(&mut self) {
        let out = self;
        out.by_file.clear();
        for (i, g) in out.glyphs.iter().enumerate() {
            if let Some(f) = g.file {
                if g.kind == K_TEXT || g.kind == K_ATOM || g.kind == K_STR {
                    out.by_file.entry(f).or_default().push((g.s, i as u32));
                }
            }
        }
        for v in out.by_file.values_mut() { v.sort(); }
    }

    pub fn text_of(&self, gi: usize) -> &str {
        let g = &self.glyphs[gi];
        let t = &self.item_text[g.item as usize];
        t.get(g.tr.0 as usize..g.tr.1 as usize).unwrap_or("")
    }
}

impl<'a> Builder<'a> {
    fn frame(&mut self, frame: &Frame, ts: Transform) {
        let mut has_text = false;
        // 글자 꾸밈 선(밑줄·취소선)은 글자와 같은 틀에 가로선으로 놓인다: (왼쪽, 오른쪽, y)
        let mut lines: Vec<(f64, f64, f64)> = vec![];
        for (pos, item) in frame.items() {
            let FrameItem::Shape(shape, span) = item else { continue };
            if !span.is_detached() || shape.stroke.is_none() { continue; }
            if let Geometry::Line(t) = &shape.geometry {
                if t.y.to_pt().abs() < 0.01 {
                    let (a, b) = (pos.x.to_pt(), pos.x.to_pt() + t.x.to_pt());
                    lines.push((a.min(b), a.max(b), pos.y.to_pt()));
                }
            }
        }
        for (pos, item) in frame.items() {
            match item {
                FrameItem::Group(group) => {
                    let ts2 = ts
                        .pre_concat(Transform::translate(pos.x, pos.y))
                        .pre_concat(group.transform);
                    self.frame(&group.frame, ts2);
                }
                FrameItem::Text(text) => {
                    if !has_text { has_text = true; self.line += 1; }
                    let item_id = self.out.item_text.len() as u32;
                    self.out.item_text.push(text.text.clone());
                    let mut x = pos.x;
                    let scale = ts.sx.get().abs() as f32;
                    let bold = self.bold_of(&text.font);
                    let fill = match &text.fill {
                        typst::visualize::Paint::Solid(c) => { let v = c.to_vec4_u8(); [v[0], v[1], v[2]] }
                        _ => [0, 0, 0],
                    };
                    // 이 글줄 조각에 그어진 선: 조각의 너비를 벗어나는 선(표의 괘선 등)은 뺀다
                    let size = text.size.to_pt();
                    let (ix0, ix1) = (pos.x.to_pt(), pos.x.to_pt() + text.width().to_pt());
                    let mut segs: Vec<(f64, f64, u8)> = vec![];
                    for &(a, b, y) in &lines {
                        if size <= 0.0 || a < ix0 - 0.3 * size || b > ix1 + 0.3 * size { continue; }
                        let dy = (y - pos.y.to_pt()) / size;
                        let bit = if (-0.05..=0.4).contains(&dy) { 1 } else if (-0.65..-0.12).contains(&dy) { 2 } else { continue };
                        segs.push((a, b, bit));
                    }
                    segs.sort_by(|p, q| (p.2, p.0).partial_cmp(&(q.2, q.0)).unwrap_or(std::cmp::Ordering::Equal));
                    // 내림 획을 피해 끊긴 선은 하나로 잇는다
                    let mut runs: Vec<(f64, f64, u8)> = vec![];
                    for (a, b, bit) in segs {
                        match runs.last_mut() {
                            Some(l) if l.2 == bit && a - l.1 <= 0.6 * size => l.1 = l.1.max(b),
                            _ => runs.push((a, b, bit)),
                        }
                    }
                    for glyph in &text.glyphs {
                        let w = glyph.x_advance.at(text.size);
                        let p0 = Point::new(x, pos.y).transform(ts);
                        let p1 = Point::new(x + w, pos.y).transform(ts);
                        let (g0, g1) = (x.to_pt(), (x + w).to_pt());
                        let mut deco = 0u8;
                        for &(a, b, bit) in &runs {
                            let ov = g1.min(b) - g0.max(a);
                            if ov > 0.0 && ov >= 0.5 * (g1 - g0) { deco |= bit; }
                        }
                        x += w;
                        let (span, off) = glyph.span;
                        let (file, ns, ne, kind, nk) = self.resolve(span);
                        let (s, e) = if kind == K_TEXT {
                            let s = ns + off as u32;
                            let e = (s + glyph.range().len() as u32).min(ne);
                            (s.min(ne), e)
                        } else { (ns, ne) };
                        self.out.glyphs.push(G {
                            page: self.page,
                            x0: p0.x.to_pt() as f32,
                            x1: p1.x.to_pt() as f32,
                            y: p0.y.to_pt() as f32,
                            size: text.size.to_pt() as f32 * scale,
                            line: self.line,
                            file, s, e, kind, node_kind: nk,
                            item: item_id,
                            tr: (glyph.range.start, glyph.range.end),
                            node: (ns, ne),
                            rec: false,
                            bold,
                            deco,
                            fill,
                        });
                    }
                }
                _ => {}
            }
        }
    }

    /// 이 글꼴로 찍힌 글자가 굵은지(가변 글꼴이면 wght 축의 값으로).
    fn bold_of(&mut self, font: &typst::text::FontInstance) -> u8 {
        const HEAVY: f32 = 600.0;
        let wght = typst::text::Tag::from_bytes(b"wght");
        let info = font.font().info();
        let weight = font.variations().0.iter().rev().find(|(t, _)| *t == wght).map(|(_, v)| v.0)
            .unwrap_or(info.variant.weight.to_number() as f32);
        if weight >= HEAVY { return 1; }
        let world = self.world;
        let can = *self.can_bold.entry(info.family.clone()).or_insert_with(|| {
            let book = world.book();
            let found = book.select_family(&info.family.to_lowercase()).filter_map(|i| book.info(i)).any(|f| {
                f.variant.weight.to_number() as f32 >= HEAVY || f.axes.iter().any(|a| a.tag == wght && a.max.0 >= HEAVY)
            });
            found
        });
        if can { 0 } else { 2 }
    }

    fn resolve(&mut self, span: Span) -> (Option<FileId>, u32, u32, u8, SyntaxKind) {
        if let Some(r) = self.cache.get(&span) { return *r; }
        let r = self.resolve_uncached(span);
        self.cache.insert(span, r);
        r
    }

    fn resolve_uncached(&self, span: Span) -> (Option<FileId>, u32, u32, u8, SyntaxKind) {
        let none = (None, 0, 0, K_NONE, SyntaxKind::End);
        let Some(id) = span.id() else { return none };
        let Ok(source) = self.world.source(id) else { return none };
        let Some(node) = source.find(span) else { return none };
        let editable_file = self.world.in_project(id);
        // 수식 안의 글자는 수식 전체($..$)를 한 덩어리로 가리킨다
        if editable_file {
            let mut p = Some(node.clone());
            while let Some(n) = p {
                if n.kind() == SyntaxKind::Equation {
                    let r = n.range();
                    return (Some(id), r.start as u32, r.end as u32, K_MATH, SyntaxKind::Equation);
                }
                p = n.parent().cloned();
            }
        }
        let r = node.range();
        let nk = node.kind();
        let kind = if !editable_file {
            K_LOCK
        } else {
            match nk {
                SyntaxKind::Text if node.parent_kind() == Some(SyntaxKind::Raw) => K_LOCK,
                SyntaxKind::Text => K_TEXT,
                SyntaxKind::Space | SyntaxKind::Escape | SyntaxKind::Shorthand
                | SyntaxKind::SmartQuote | SyntaxKind::Linebreak => K_ATOM,
                _ => K_LOCK,
            }
        };
        (Some(id), r.start as u32, r.end as u32, kind, nk)
    }
}
