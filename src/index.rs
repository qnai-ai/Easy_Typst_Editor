//! 지면 색인: 조판된 모든 글자(글리프)가 지면의 어디에 있고, 원본의 어느 바이트에서 왔는지 기록한다.

use std::collections::HashMap;

use typst::layout::{Frame, FrameItem, Point, Transform};
use typst::syntax::{FileId, Span, SyntaxKind};
use typst::World;
use typst_layout::PagedDocument;

use crate::world::ProjectWorld;

/// 글자 종류.
pub const K_TEXT: u8 = 0; // 마크업 본문 글자: 글자 단위로 편집
pub const K_ATOM: u8 = 1; // 공백·이스케이프·줄임표 등: 통째로 다루는 한 덩어리
pub const K_LOCK: u8 = 2; // 원본 위치는 알지만 지면에서 고칠 수 없음(코드·문자열·수식 등)
pub const K_NONE: u8 = 3; // 원본 위치를 알 수 없음(자동 생성)
pub const K_STR: u8 = 4; // 따옴표 문자열 안의 글자(제목 등): 글자만 고칠 수 있음

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
}

impl Index {
    pub fn build(world: &ProjectWorld, doc: &PagedDocument, reuse: Option<Reuse>) -> Index {
        let mut b = Builder { world, cache: HashMap::new(), out: Index::default(), line: 0, page: 0 };
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
        out.index_files();
        crate::recover::recover(world, &mut out, &changed);
        out.index_files();
        out
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
                    for glyph in &text.glyphs {
                        let w = glyph.x_advance.at(text.size);
                        let p0 = Point::new(x, pos.y).transform(ts);
                        let p1 = Point::new(x + w, pos.y).transform(ts);
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
                        });
                    }
                }
                _ => {}
            }
        }
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
        let r = node.range();
        let nk = node.kind();
        let editable_file = self.world.in_project(id);
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
