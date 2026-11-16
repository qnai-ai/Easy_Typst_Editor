//! 편집 세션: 열린 교재 하나의 조판 결과, 커서, 선택, 실행 취소, 저장.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};
use typst::diag::{Severity, SourceDiagnostic};
use typst::model::Document;
use typst::syntax::FileId;
use typst::WorldExt;
use typst_kit::fonts::FontStore;
use typst_layout::PagedDocument;

use crate::edit::{self, Style, Work};
use crate::index::{Index, Reuse, G, K_ATOM, K_LOCK, K_NONE, K_STR, K_TEXT};
use crate::world::ProjectWorld;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Pos { pub file: FileId, pub off: usize }

#[derive(Clone, Copy, Debug)]
struct Caret {
    pos: Pos,
    aff_end: bool,
    gi: Option<usize>,
    page: u32,
    x: f32,
    y: f32,
    size: f32,
}

struct ElemSel {
    file: FileId,
    range: std::ops::Range<usize>,
    name: String,
    /// 고를 때 누른 자리(쪽, x, y): 같은 요소가 여러 곳에 찍혔을 때 어느 것인지 가린다
    at: (u32, f32, f32),
    /// 이 요소의 상자(너비를 끌어 바꿀 수 있을 때)
    bx: Option<BoxSel>,
}

/// 지면의 상자 하나: 조판 결과의 프레임 묶음과, 그것을 만든 원고 자리(알 수 있을 때).
struct Bx {
    page: u32, x: f32, y: f32, w: f32, h: f32,
    depth: u32,
    /// 이 상자 안에 든 글리프의 색인 범위
    g0: usize, g1: usize,
    /// 채움·테두리를 그린 요소(block·box·rect …)의 원고 자리
    shape: Option<(FileId, std::ops::Range<usize>)>,
    /// 표·격자면 그 원고 자리
    grid: Option<(FileId, std::ops::Range<usize>)>,
    /// 표·격자의 칸마다 왼쪽 끝(지면 좌표)과 칸 사이 간격
    lefts: Vec<f32>,
    gutter: f32,
}

impl Bx {
    fn has(&self, x: f32, y: f32) -> bool { x >= self.x - 1.0 && x <= self.x + self.w + 1.0 && y >= self.y - 1.0 && y <= self.y + self.h + 1.0 }
    fn dist(&self, x: f32, y: f32) -> f32 {
        let dx = (self.x - x).max(x - self.x - self.w).max(0.0);
        let dy = (self.y - y).max(y - self.y - self.h).max(0.0);
        dx + dy
    }
    /// 칸마다의 너비. 칸 자리를 다 알지 못하면 빈 목록.
    fn col_widths(&self) -> Vec<f32> {
        let n = self.lefts.len();
        if n == 0 || self.lefts.iter().any(|l| l.is_nan()) { return vec![]; }
        (0..n).map(|i| if i + 1 < n { self.lefts[i + 1] - self.lefts[i] - self.gutter } else { self.x + self.w - self.lefts[i] }).collect()
    }
}

/// 골라 둔 요소에 딸린 상자: 손잡이를 그릴 자리와 고칠 원고 자리.
struct BoxSel {
    file: FileId,
    range: std::ops::Range<usize>,
    name: String,
    grid: bool,
    page: u32, x: f32, y: f32, w: f32, h: f32,
    cols: Vec<f32>,
    /// 칸 사이 경계의 x(지면 좌표)
    bounds: Vec<f32>,
    /// 같은 원고 자리에서 나온 상자의 수(템플릿 요소면 여러 곳)
    copies: usize,
}

struct BoxWalk<'a> {
    world: &'a ProjectWorld,
    page: u32,
    g: usize,
    out: Vec<Bx>,
    /// 방금 지나온 표·격자 표식: 바로 다음 프레임 묶음이 그 표다
    pending: Option<((FileId, std::ops::Range<usize>), f32)>,
}

impl BoxWalk<'_> {
    fn src(&self, span: typst::syntax::Span) -> Option<(FileId, std::ops::Range<usize>)> {
        let id = span.id()?;
        if !self.world.in_project(id) { return None; }
        Some((id, self.world.range(span)?))
    }

    fn frame(&mut self, frame: &typst::layout::Frame, ts: typst::layout::Transform, me: Option<usize>, grid: Option<usize>, depth: u32) {
        use typst::foundations::Value;
        use typst::introspection::Tag;
        use typst::layout::{FrameItem, Point, Transform};
        for (pos, item) in frame.items() {
            let pending = self.pending.take();
            match item {
                FrameItem::Group(group) => {
                    let ts2 = ts.pre_concat(Transform::translate(pos.x, pos.y)).pre_concat(group.transform);
                    let a = Point::zero().transform(ts2);
                    let b = Point::new(group.frame.width(), group.frame.height()).transform(ts2);
                    let (ax, ay, bx, by) = (a.x.to_pt() as f32, a.y.to_pt() as f32, b.x.to_pt() as f32, b.y.to_pt() as f32);
                    let idx = self.out.len();
                    self.out.push(Bx {
                        page: self.page, x: ax.min(bx), y: ay.min(by), w: (bx - ax).abs(), h: (by - ay).abs(), depth,
                        g0: self.g, g1: self.g, shape: None,
                        grid: pending.as_ref().map(|p| p.0.clone()), lefts: vec![], gutter: pending.map(|p| p.1).unwrap_or(0.0),
                    });
                    let inner = if self.out[idx].grid.is_some() { Some(idx) } else { grid };
                    self.frame(&group.frame, ts2, Some(idx), inner, depth + 1);
                    self.out[idx].g1 = self.g;
                }
                FrameItem::Text(t) => self.g += t.glyphs.len(),
                // 상자의 바탕(채움·테두리): 상자와 너비가 같고 왼쪽 끝에서 시작하는 도형
                FrameItem::Shape(shape, span) => {
                    if let Some(i) = me {
                        if self.out[i].shape.is_none() {
                            let bb = shape.geometry.bbox(None);
                            let sw = (bb.max.x - bb.min.x).to_pt() as f32;
                            let fw = frame.width().to_pt() as f32;
                            if (pos.x + bb.min.x).to_pt().abs() < 1.5 && (sw - fw).abs() < 2.0 {
                                self.out[i].shape = self.src(*span);
                            }
                        }
                    }
                }
                FrameItem::Tag(Tag::Start(c, _)) => match c.func().name() {
                    "grid" | "table" => {
                        // 칸 사이 간격(첫 값만 본다)
                        let gutter = match c.get_by_name("column-gutter") {
                            Ok(Value::Array(a)) => match a.iter().next() {
                                Some(Value::Length(l)) => l.abs.to_pt() as f32,
                                Some(Value::Relative(r)) => r.abs.abs.to_pt() as f32,
                                _ => 0.0,
                            },
                            _ => 0.0,
                        };
                        self.pending = self.src(c.span()).map(|s| (s, gutter));
                    }
                    "cell" => {
                        if let (Some(gi), Ok(Value::Int(x))) = (grid, c.get_by_name("x")) {
                            let px = Point::new(pos.x, pos.y).transform(ts).x.to_pt() as f32;
                            let v = &mut self.out[gi].lefts;
                            let x = x.max(0) as usize;
                            if x < 64 {
                                if v.len() <= x { v.resize(x + 1, f32::NAN); }
                                if v[x].is_nan() || px < v[x] { v[x] = px; }
                            }
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
}

struct Lock { page: u32, x: f32, y: f32, w: f32, h: f32, reason: String }

struct Step {
    file: FileId,
    before: String,
    after: String,
    caret_before: Option<Pos>,
    caret_after: Option<Pos>,
    /// 이어지는 같은 갈래의 걸음은 하나로 합친다: 0 = 따로, 1 = 글자 입력, 2 = 글자 지우기
    kind: u8,
    time: Instant,
}

/// 찾기에서 줄이 바뀌는 자리(공백 없이 줄만 넘어간 곳)를 나타내는 표식.
const SOFT: char = '\u{1}';

struct Comp { file: FileId, base_text: String, base_caret: usize, base_anchor: usize, caret_before: Option<Pos> }

pub struct Problem { pub title: String, pub detail: String, pub hint: String }

pub struct Session {
    pub world: ProjectWorld,
    doc: PagedDocument,
    ix: Index,
    hashes: Vec<String>,
    pub main_path: PathBuf,
    pub title: String,
    caret: Option<Caret>,
    anchor: Option<Pos>,
    lock: Option<Lock>,
    elem: Option<ElemSel>,
    undo: Vec<Step>,
    redo: Vec<Step>,
    comp: Option<Comp>,
    goal_x: Option<f32>,
    msg: Option<(String, &'static str)>,
    pub warnings: Vec<Problem>,
    rev: u64,
    last_ms: u64,
    timing: String,
    ix_src: Vec<(FileId, String)>,
    under_fn: String,
}

fn editable(g: &G) -> bool { (g.kind == K_TEXT || g.kind == K_ATOM || g.kind == K_STR) && g.e > g.s }
fn charlevel(g: &G) -> bool { g.kind == K_TEXT || g.kind == K_STR }

pub fn translate(world: &ProjectWorld, d: &SourceDiagnostic) -> Problem {
    let m = d.message.to_string();
    let mut place = String::new();
    if let Some(id) = d.span.id() {
        if let (Some(src), Some(r)) = (world.source_of(id), world.range(d.span)) {
            if let Some(line) = src.lines().byte_to_line(r.start) {
                place = crate::lang::f("{0} {1}행", &[&id.vpath().get_without_slash(), &(line + 1)]);
            }
        }
    }
    let (title, hint) = if m.contains("file not found") {
        ("조판에 필요한 파일을 찾지 못했습니다.", "main.typ가 있는 폴더(또는 그 아래 폴더)에 이 파일이 있는지, 이름이 바뀌지 않았는지 확인해 주세요.")
    } else if m.contains("unknown font family") {
        ("교재에 쓰인 글꼴이 이 컴퓨터에 없습니다.", "글꼴을 설치한 뒤 교재를 다시 열어 주세요. 지금은 다른 글꼴로 대신 짜여 쪽 나눔이 달라질 수 있습니다.")
    } else if m.contains("access denied") || m.contains("project root") || m.contains("outside") {
        ("교재 폴더 밖의 파일을 불러오려 했습니다.", "main.typ가 교재 폴더의 맨 위에 있는지 확인해 주세요.")
    } else if m.contains("package") || m.contains("패키지") {
        ("교재가 쓰는 Typst 패키지가 이 컴퓨터에 없습니다.", "교재가 쓰는 공통 패키지를 이 컴퓨터의 typst에 이어 준 뒤(교재 폴더의 link.ps1 같은 설치 스크립트 실행, 또는 평소 방식으로 한 번 조판) 다시 열어 주세요.")
    } else if m.contains("failed to decode image") || m.contains("unknown image format") {
        ("그림 파일을 읽지 못했습니다.", "그림 파일이 깨지지 않았는지 확인해 주세요.")
    } else {
        ("원고에 조판 오류가 있습니다.", "원고(.typ)를 평소 쓰는 편집기에서 고친 뒤 다시 열어 주세요.")
    };
    let mut detail = m;
    if !place.is_empty() { detail = format!("{detail}  ({place})"); }
    for h in d.hints.iter() { detail.push_str(&format!("  · {}", h.v)); }
    Problem { title: title.into(), detail, hint: hint.into() }
}

impl Session {
    pub fn open(main: &Path, fonts: Arc<FontStore>) -> Result<Session, Vec<Problem>> {
        let main = main.canonicalize().unwrap_or(main.to_path_buf());
        let main = PathBuf::from(main.to_string_lossy().trim_start_matches(r"\\?\").to_string());
        if !main.is_file() {
            return Err(vec![Problem { title: "파일을 찾지 못했습니다.".into(), detail: main.display().to_string(), hint: "파일이 옮겨졌거나 지워졌는지 확인해 주세요.".into() }]);
        }
        let mut root = main.parent().unwrap().to_path_buf();
        let mut last_err = vec![];
        for _ in 0..3 {
            let world = match ProjectWorld::new(&root, &main, fonts.clone()) {
                Ok(w) => w,
                Err(e) => return Err(vec![Problem { title: e, detail: String::new(), hint: String::new() }]),
            };
            let res = typst::compile::<PagedDocument>(&world);
            match res.output {
                Ok(doc) => {
                    let warnings = res.warnings.iter().filter(|w| w.severity == Severity::Warning)
                        .map(|w| translate(&world, w)).collect::<Vec<_>>();
                    let ix = Index::build(&world, &doc, None);
                    let title = doc.info().title.as_ref().map(|t| t.to_string()).filter(|t| !t.is_empty())
                        .unwrap_or_else(|| root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
                    let mut s = Session {
                        world, doc, ix, hashes: vec![], main_path: main.clone(), title,
                        caret: None, anchor: None, lock: None, elem: None, undo: vec![], redo: vec![], comp: None,
                        goal_x: None, msg: None, warnings, rev: 0, last_ms: 0, timing: String::new(), ix_src: vec![], under_fn: String::new(),
                    };
                    s.hashes = Self::hash_pages(&s.doc);
                    s.under_fn = s.world.underline_fn();
                    if let Some(w) = s.warnings.first() {
                        s.msg = Some((format!("{} {}", w.title, w.detail), "warn"));
                    }
                    return Ok(s);
                }
                Err(errs) => {
                    let outside = errs.iter().any(|e| {
                        let m = e.message.as_str();
                        m.contains("access denied") || m.contains("project root") || m.contains("outside")
                    });
                    last_err = errs.iter().map(|e| translate(&world, e)).collect();
                    if outside {
                        if let Some(p) = root.parent() { root = p.to_path_buf(); continue; }
                    }
                    break;
                }
            }
        }
        Err(last_err)
    }

    /// 파일 본문을 바꾼다. 지금 색인이 어느 본문으로 만들어졌는지 기억해 둔다.
    fn set_text(&mut self, file: FileId, text: &str) {
        if !self.ix_src.iter().any(|(f, _)| *f == file) {
            if let Some(old) = self.world.text_of(file) { self.ix_src.push((file, old)); }
        }
        self.world.set_text(file, text);
    }

    fn hash_pages(doc: &PagedDocument) -> Vec<String> {
        doc.pages().iter().map(|p| {
            let h = typst::utils::hash128(&p.frame);
            format!("{:x}", (h as u64) ^ ((h >> 64) as u64))
        }).collect()
    }

    fn compile(&mut self) -> Result<(), String> {
        let t0 = Instant::now();
        let res = typst::compile::<PagedDocument>(&self.world);
        let t1 = t0.elapsed();
        match res.output {
            Ok(doc) => {
                let hashes = Self::hash_pages(&doc);
                // 고친 자리 뒤의 글자들은 원본 위치가 밀린다
                let mut shifts = vec![];
                for (file, old) in self.ix_src.drain(..) {
                    let Some(new) = self.world.text_of(file) else { continue };
                    if new == old { continue; }
                    let (ob, nb) = (old.as_bytes(), new.as_bytes());
                    let mut pre = 0;
                    while pre < ob.len() && pre < nb.len() && ob[pre] == nb[pre] { pre += 1; }
                    let mut suf = 0;
                    while suf < ob.len() - pre && suf < nb.len() - pre && ob[ob.len() - 1 - suf] == nb[nb.len() - 1 - suf] { suf += 1; }
                    shifts.push((file, (ob.len() - suf) as u32, nb.len() as i64 - ob.len() as i64));
                }
                let ix = Index::build(&self.world, &doc, Some(Reuse { old: &self.ix, old_hashes: &self.hashes, new_hashes: &hashes, shifts }));
                let t2 = t0.elapsed();
                self.ix = ix;
                self.doc = doc;
                self.hashes = hashes;
                self.rev += 1;
                if self.rev % 12 == 0 { typst::comemo::evict(30); }
                let t3 = t0.elapsed();
                self.timing = crate::lang::f("조판 {0}ms · 색인 {1}ms · 정리 {2}ms", &[&t1.as_millis(), &(t2 - t1).as_millis(), &(t3 - t2).as_millis()]);
                Ok(())
            }
            Err(errs) => Err(errs.first().map(|e| e.message.to_string()).unwrap_or_default()),
        }
    }

    pub fn is_dirty(&self) -> bool { !self.world.dirty_files().is_empty() }

    // ───────────── 위치 찾기

    fn glyph(&self, gi: usize) -> &G { &self.ix.glyphs[gi] }

    fn caret_on(&self, gi: usize, off: usize) -> Caret {
        let g = self.glyph(gi);
        let (s, e) = (g.s as usize, g.e as usize);
        let x = if off <= s { g.x0 } else if off >= e { g.x1 } else {
            g.x0 + (g.x1 - g.x0) * (off - s) as f32 / (e - s) as f32
        };
        Caret { pos: Pos { file: g.file.unwrap(), off }, aff_end: off >= e && e > s, gi: Some(gi), page: g.page, x, y: g.y, size: g.size }
    }

    fn locate(&self, pos: Pos, aff_end: bool, hint: Option<(u32, f32, f32)>) -> Caret {
        let none = Caret { pos, aff_end, gi: None, page: hint.map(|h| h.0).unwrap_or(0), x: hint.map(|h| h.1).unwrap_or(0.0), y: hint.map(|h| h.2).unwrap_or(0.0), size: 10.0 };
        let Some(v) = self.ix.by_file.get(&pos.file) else { return none };
        let off = pos.off as u32;
        let i0 = v.partition_point(|x| x.0 < off);
        let (mut a, mut b, mut c) = (vec![], vec![], vec![]);
        for &(s, gi) in &v[i0..] { if s != off { break; } if self.glyph(gi as usize).e > s { a.push(gi as usize); } }
        let mut j = i0;
        let mut steps = 0;
        while j > 0 && steps < 16 {
            j -= 1; steps += 1;
            let g = self.glyph(v[j].1 as usize);
            if g.e == off && g.e > g.s { b.push(v[j].1 as usize); } else if g.e > off { c.push(v[j].1 as usize); }
        }
        let score = |gi: &usize| {
            let g = self.glyph(*gi);
            match hint {
                Some((p, x, y)) => (g.page as f32 - p as f32).abs() * 100000.0 + (g.y - y).abs() * 3.0 + (g.x0 - x).abs() * 0.01,
                None => *gi as f32,
            }
        };
        let best = |v: &Vec<usize>| v.iter().copied().min_by(|x, y| score(x).partial_cmp(&score(y)).unwrap());
        let order: [&Vec<usize>; 3] = if aff_end { [&b, &a, &c] } else { [&a, &b, &c] };
        for set in order {
            if let Some(gi) = best(set) { return self.caret_on(gi, pos.off); }
        }
        // 글자가 없는 자리(탭 뒤, 빈 문단 등): 가장 가까운 앞 글자의 오른쪽
        if i0 > 0 {
            // 원고에서 바로 앞에 있는 글자(같은 글자가 여러 곳에 찍혔으면 그중 가까운 것)
            let s_last = v[i0 - 1].0;
            let from = v[..i0].partition_point(|x| x.0 < s_last);
            let cands: Vec<usize> = v[from..i0].iter().map(|x| x.1 as usize).collect();
            if let Some(gi) = best(&cands).or(cands.last().copied()) {
                let g = self.glyph(gi);
                let mut cr = self.caret_on(gi, g.e as usize);
                if let Some(t) = self.world.text_of(pos.file) { cr.x += edit::tabs_between(&t, g.e as usize, pos.off); }
                cr.pos = pos; cr.aff_end = true;
                return cr;
            }
        }
        if let Some(&(_, gi)) = v.get(i0) {
            let mut cr = self.caret_on(gi as usize, self.glyph(gi as usize).s as usize);
            cr.pos = pos;
            return cr;
        }
        none
    }

    fn hit(&self, page: usize, x: f32, y: f32) -> Option<(usize, bool)> {
        let &(a, b) = self.ix.page_ranges.get(page)?;
        let mut best: Option<(usize, f32)> = None;
        let mut best_e: Option<(usize, f32)> = None;
        for gi in a..b {
            let g = &self.ix.glyphs[gi];
            let (top, bot) = (g.y - g.size * 0.92, g.y + g.size * 0.36);
            let dy = if y < top { top - y } else if y > bot { y - bot } else { 0.0 };
            if dy > 30.0 { continue; }
            let dx = if x < g.x0 { g.x0 - x } else if x > g.x1 { x - g.x1 } else { 0.0 };
            let sc = dy * 6.0 + dx;
            if best.map_or(true, |b| sc < b.1) { best = Some((gi, sc)); }
            if editable(g) && best_e.map_or(true, |b| sc < b.1) { best_e = Some((gi, sc)); }
        }
        let (gi, sc) = best?;
        if sc == 0.0 { return Some((gi, true)); }
        match best_e { Some((ge, _)) => Some((ge, false)), None => Some((gi, false)) }
    }

    fn char_bounds(&self, g: &G) -> Vec<usize> {
        let (s, e) = (g.s as usize, g.e as usize);
        let mut v = vec![s];
        if charlevel(g) {
            if let Some(t) = self.world.text_of(g.file.unwrap()) {
                if let Some(sl) = t.get(s..e) { for (o, _) in sl.char_indices().skip(1) { v.push(s + o); } }
            }
        }
        v.push(e);
        v
    }

    fn hint(&self) -> Option<(u32, f32, f32)> { self.caret.map(|c| (c.page, c.x, c.y)) }

    // ───────────── 마우스

    fn click(&mut self, page: usize, x: f32, y: f32, extend: bool, dbl: bool, tpl: bool) {
        self.goal_x = None;
        let Some((gi, _inside)) = self.hit(page, x, y) else {
            if !extend { self.lock = None; }
            return;
        };
        let g = self.glyph(gi).clone();
        if !editable(&g) {
            if extend { return; }
            // 같은 줄에서 이어진 못 고치는 글자들을 묶어 보여 준다
            let (mut a, mut b) = (gi, gi);
            while a > 0 { let p = self.glyph(a - 1); if p.line == g.line && !editable(p) { a -= 1; } else { break; } }
            while b + 1 < self.ix.glyphs.len() { let n = self.glyph(b + 1); if n.line == g.line && !editable(n) { b += 1; } else { break; } }
            let (ga, gb) = (self.glyph(a), self.glyph(b));
            let reason = if g.kind == K_LOCK || g.kind == K_NONE {
                "템플릿이 자동으로 만들어 넣는 글(쪽 번호·차례·머리말·꾸민 표제어·갈래 이름 등)이어서, 원고에서 고칠 자리를 정확히 찾을 수 없습니다.".to_string()
            } else { "이 글자는 지면에서 고칠 수 없습니다.".to_string() };
            self.lock = Some(Lock { page: g.page, x: ga.x0.min(gb.x0), y: g.y - g.size * 0.9, w: (gb.x1 - ga.x0).abs().max(2.0), h: g.size * 1.25, reason });
            self.caret = None; self.anchor = None;
            return;
        }
        self.lock = None;
        let bounds = self.char_bounds(&g);
        let frac = if g.x1 > g.x0 { ((x - g.x0) / (g.x1 - g.x0)).clamp(0.0, 1.0) } else { 0.0 };
        let idx = (frac * (bounds.len() - 1) as f32).round() as usize;
        let off = bounds[idx.min(bounds.len() - 1)];
        let new = self.caret_on(gi, off);
        if tpl {
            if let Some(t) = self.world.text_of(new.pos.file) {
                if let Some((s, e)) = edit::para_range(&t, off) {
                    self.caret = Some(new);
                    self.select(new.pos.file, s, e);
                    return;
                }
            }
        }
        if dbl && charlevel(&g) {
            if let Some(t) = self.world.text_of(new.pos.file) {
                let (ws, we) = edit::word_at(&t, off.min(g.e as usize - 1).max(g.s as usize));
                let (ws, we) = (ws.max(g.node_start(&t)), we);
                if we > ws {
                    self.anchor = Some(Pos { file: new.pos.file, off: ws });
                    self.caret = Some(self.locate(Pos { file: new.pos.file, off: we }, true, Some((g.page, x, g.y))));
                    return;
                }
            }
        }
        if extend {
            let base = self.anchor.or(self.caret.map(|c| c.pos));
            match base {
                Some(b) if b.file == new.pos.file => { self.anchor = Some(b); }
                Some(_) => { self.msg = Some(("다른 원고 파일에 걸쳐서는 선택할 수 없습니다.".into(), "warn")); return; }
                None => {}
            }
        } else { self.anchor = None; }
        self.caret = Some(new);
        if self.anchor == Some(new.pos) { self.anchor = None; }
    }

    // ───────────── 글자판 이동

    fn next_editable(&self, gi: usize, fwd: bool) -> Option<usize> {
        let n = self.ix.glyphs.len();
        let mut i = gi;
        for _ in 0..6000 {
            if fwd { i += 1; if i >= n { return None; } } else { if i == 0 { return None; } i -= 1; }
            if editable(&self.ix.glyphs[i]) { return Some(i); }
        }
        None
    }

    fn step(&self, c: &Caret, fwd: bool) -> Option<Caret> {
        let gi = c.gi?;
        let g = self.glyph(gi);
        let same = g.file == Some(c.pos.file);
        let (s, e) = (g.s as usize, g.e as usize);
        let within = |g: &G, gi: usize, off: usize, fwd: bool| -> Caret {
            let b = self.char_bounds(g);
            let t = if fwd { b.iter().copied().find(|&x| x > off).unwrap_or(g.e as usize) }
                    else { b.iter().rev().copied().find(|&x| x < off).unwrap_or(g.s as usize) };
            self.caret_on(gi, t)
        };
        if fwd {
            if same && c.pos.off >= s && c.pos.off < e { return Some(within(g, gi, c.pos.off, true)); }
            let n = self.next_editable(gi, true)?;
            let ng = self.glyph(n);
            let contiguous = ng.file == Some(c.pos.file) && ng.s as usize == c.pos.off;
            let same_spot = ng.line == g.line && (ng.x0 - c.x).abs() < 0.05;
            if contiguous || same_spot { Some(within(ng, n, ng.s as usize, true)) } else { Some(self.caret_on(n, ng.s as usize)) }
        } else {
            if same && c.pos.off > s && c.pos.off <= e { return Some(within(g, gi, c.pos.off, false)); }
            let p = self.next_editable(gi, false)?;
            let pg = self.glyph(p);
            let contiguous = pg.file == Some(c.pos.file) && pg.e as usize == c.pos.off;
            let same_spot = pg.line == g.line && (pg.x1 - c.x).abs() < 0.05;
            if contiguous || same_spot { Some(within(pg, p, pg.e as usize, false)) } else { Some(self.caret_on(p, pg.e as usize)) }
        }
    }

    fn line_span(&self, gi: usize) -> (usize, usize) {
        let line = self.glyph(gi).line;
        let (mut a, mut b) = (gi, gi);
        while a > 0 && self.glyph(a - 1).line == line { a -= 1; }
        while b + 1 < self.ix.glyphs.len() && self.glyph(b + 1).line == line { b += 1; }
        (a, b)
    }

    fn vertical(&self, c: &Caret, down: bool) -> Option<Caret> {
        let goal = self.goal_x.unwrap_or(c.x);
        let mut page = c.page as usize;
        let mut cur_y = c.y;
        for _ in 0..3 {
            let &(a, b) = self.ix.page_ranges.get(page)?;
            // 줄마다 (y, xmin, xmax, 첫 글리프, 끝 글리프)
            let mut best: Option<(f32, usize, usize)> = None;
            let mut i = a;
            while i < b {
                let line = self.ix.glyphs[i].line;
                let mut j = i;
                let (mut x0, mut x1, mut any) = (f32::MAX, f32::MIN, false);
                while j < b && self.ix.glyphs[j].line == line {
                    let g = &self.ix.glyphs[j];
                    if editable(g) { any = true; x0 = x0.min(g.x0); x1 = x1.max(g.x1); }
                    j += 1;
                }
                if any {
                    let y = self.ix.glyphs[i].y;
                    let dy = if down { y - cur_y } else { cur_y - y };
                    if dy > 0.6 {
                        let dx = if goal < x0 - 4.0 { x0 - goal } else if goal > x1 + 4.0 { goal - x1 } else { 0.0 };
                        let sc = dy + dx * 3.0;
                        if best.map_or(true, |bst| sc < bst.0) { best = Some((sc, i, j)); }
                    }
                }
                i = j;
            }
            if let Some((_, i, j)) = best {
                let mut pick: Option<(f32, usize)> = None;
                for k in i..j {
                    let g = &self.ix.glyphs[k];
                    if !editable(g) { continue; }
                    let d = if goal < g.x0 { g.x0 - goal } else if goal > g.x1 { goal - g.x1 } else { 0.0 };
                    if pick.map_or(true, |p| d < p.0) { pick = Some((d, k)); }
                }
                let (_, k) = pick?;
                let g = &self.ix.glyphs[k];
                let bounds = self.char_bounds(g);
                let frac = if g.x1 > g.x0 { ((goal - g.x0) / (g.x1 - g.x0)).clamp(0.0, 1.0) } else { 0.0 };
                let idx = (frac * (bounds.len() - 1) as f32).round() as usize;
                return Some(self.caret_on(k, bounds[idx]));
            }
            if down { page += 1; cur_y = -1.0e6; } else { if page == 0 { return None; } page -= 1; cur_y = 1.0e6; }
        }
        None
    }

    /// Ctrl+←/→: 낱말 단위로 옮긴다. 왼쪽은 낱말의 처음으로, 오른쪽은 다음 낱말의 처음으로.
    fn word_step(&self, c: &Caret, fwd: bool) -> Option<Caret> {
        let text = self.world.text_of(c.pos.file)?;
        let class = |ch: char| if ch.is_whitespace() { 0u8 } else if ch.is_alphanumeric() || ch == '\'' || ch == '’' { 2 } else { 1 };
        let mut cur = *c;
        let (mut seen, mut spaced, mut moved) = (0u8, false, false);
        for _ in 0..300 {
            let Some(nx) = self.step(&cur, fwd) else { break };
            let (lo, hi) = if fwd { (cur.pos.off, nx.pos.off) } else { (nx.pos.off, cur.pos.off) };
            let chunk = if nx.pos.file == cur.pos.file && hi > lo && hi - lo <= 64 { text.get(lo..hi) } else { None };
            // 글이 이어지지 않는 자리(다른 글 상자): 아직 안 움직였으면 거기까지만
            let Some(chunk) = chunk else { if !moved { cur = nx; moved = true; } break };
            let cls = if fwd { chunk.chars().next() } else { chunk.chars().next_back() }.map(class).unwrap_or(0);
            if cls == 0 {
                if !fwd && seen != 0 { break; }
                spaced = true;
            } else {
                if fwd && spaced && moved { break; }
                if seen != 0 && seen != cls { break; }
                seen = cls;
            }
            cur = nx; moved = true;
        }
        moved.then_some(cur)
    }

    fn doc_end(&self, last: bool) -> Option<Caret> {
        let n = self.ix.glyphs.len();
        if last { (0..n).rev().find(|&i| editable(self.glyph(i))).map(|i| self.caret_on(i, self.glyph(i).e as usize)) }
        else { (0..n).find(|&i| editable(self.glyph(i))).map(|i| self.caret_on(i, self.glyph(i).s as usize)) }
    }

    /// Ctrl+↑/↓: 문단의 처음으로 / 다음 문단의 처음으로.
    fn para_step(&self, c: &Caret, down: bool) -> Option<Caret> {
        let text = self.world.text_of(c.pos.file)?;
        let (s, e) = edit::para_range(&text, c.pos.off)?;
        if down {
            let end = self.locate(Pos { file: c.pos.file, off: e }, true, self.hint());
            return self.step(&end, true);
        }
        if c.pos.off > s { return Some(self.locate(Pos { file: c.pos.file, off: s }, false, self.hint())); }
        let prev = self.step(c, false)?;
        let t2 = self.world.text_of(prev.pos.file)?;
        let (s2, _) = edit::para_range(&t2, prev.pos.off)?;
        Some(self.locate(Pos { file: prev.pos.file, off: s2 }, false, Some((prev.page, prev.x, prev.y))))
    }

    fn mv(&mut self, dir: &str, shift: bool) {
        let Some(c) = self.caret else { return };
        let sel = self.sel_range();
        let new = match dir {
            "left" => {
                if !shift && sel.is_some() { let (a, _) = sel.unwrap(); Some(self.locate(Pos { file: c.pos.file, off: a }, false, self.hint())) }
                else { self.step(&c, false) }
            }
            "right" => {
                if !shift && sel.is_some() { let (_, b) = sel.unwrap(); Some(self.locate(Pos { file: c.pos.file, off: b }, true, self.hint())) }
                else { self.step(&c, true) }
            }
            "wordleft" => self.word_step(&c, false),
            "wordright" => self.word_step(&c, true),
            "docstart" => self.doc_end(false),
            "docend" => self.doc_end(true),
            "paraup" => self.para_step(&c, false),
            "paradown" => self.para_step(&c, true),
            "up" => self.vertical(&c, false),
            "down" => self.vertical(&c, true),
            "home" | "end" => c.gi.and_then(|gi| {
                let (a, b) = self.line_span(gi);
                if dir == "home" { (a..=b).find(|&i| editable(self.glyph(i))).map(|i| self.caret_on(i, self.glyph(i).s as usize)) }
                else { (a..=b).rev().find(|&i| editable(self.glyph(i))).map(|i| self.caret_on(i, self.glyph(i).e as usize)) }
            }),
            _ => None,
        };
        if matches!(dir, "up" | "down") { if self.goal_x.is_none() { self.goal_x = Some(c.x); } } else { self.goal_x = None; }
        let Some(new) = new else { return };
        if shift {
            if new.pos.file != c.pos.file {
                self.msg = Some(("다른 원고 파일에 걸쳐서는 선택할 수 없습니다.".into(), "warn"));
                return;
            }
            if self.anchor.is_none() { self.anchor = Some(c.pos); }
        } else { self.anchor = None; }
        self.caret = Some(new);
        if self.anchor == Some(new.pos) { self.anchor = None; }
        self.lock = None;
    }

    /// 원고의 구간을 선택한다(시작 = 고정점, 끝 = 커서).
    fn select(&mut self, file: FileId, s: usize, e: usize) {
        let hint = self.hint();
        self.anchor = Some(Pos { file, off: s });
        self.caret = Some(self.locate(Pos { file, off: e }, true, hint));
        if s == e { self.anchor = None; }
        self.lock = None;
    }

    /// Ctrl+A: 커서가 든 글 상자 전체를 선택한다.
    fn select_all(&mut self) {
        let Some(c) = self.caret else {
            self.msg = Some(("먼저 고칠 자리를 눌러 커서를 놓아 주세요.".into(), "info"));
            return;
        };
        let Some(text) = self.world.text_of(c.pos.file) else { return };
        if let Some((s, e)) = edit::block_range(&text, c.pos.off) { self.select(c.pos.file, s, e); }
    }

    // ───────────── 찾기·바꾸기

    /// 지면에서 글을 찾는다. 돌려주는 것은 찾은 곳마다 걸린 글리프들(지면 순서).
    /// 공백은 몇 칸이든 한 칸으로 보고, 줄이 바뀌는 자리는 공백이 있든 없든 맞는 것으로 본다.
    fn hits(&self, q: &str, case: bool) -> Vec<Vec<usize>> {
        let fold = |c: char| if case { c } else { c.to_lowercase().next().unwrap_or(c) };
        let mut pat: Vec<char> = vec![];
        for c in q.trim().chars() {
            if c.is_whitespace() { if pat.last() != Some(&' ') { pat.push(' '); } } else { pat.push(fold(c)); }
        }
        let mut out = vec![];
        if pat.is_empty() { return out; }
        for &(a, b) in &self.ix.page_ranges {
            let mut s: Vec<(char, usize)> = Vec::with_capacity(b - a + 16);
            let mut line = u32::MAX;
            for gi in a..b {
                let g = &self.ix.glyphs[gi];
                if g.line != line {
                    if line != u32::MAX && s.last().is_some_and(|l| l.0 != ' ') { s.push((SOFT, gi)); }
                    line = g.line;
                }
                for c in self.ix.text_of(gi).chars() {
                    if c.is_whitespace() {
                        match s.last_mut() {
                            Some(l) if l.0 == SOFT => l.0 = ' ',
                            Some(l) if l.0 != ' ' => s.push((' ', gi)),
                            _ => {}
                        }
                    } else { s.push((fold(c), gi)); }
                }
            }
            let mut i = 0;
            while i < s.len() {
                if s[i].0 != pat[0] { i += 1; continue; }
                let (mut x, mut y) = (i, 0);
                let mut gis: Vec<usize> = vec![];
                while y < pat.len() && x < s.len() {
                    let (c, gi) = s[x];
                    if c == SOFT { if pat[y] == ' ' { y += 1; } x += 1; continue; }
                    if c != pat[y] { break; }
                    if c != ' ' && gis.last() != Some(&gi) { gis.push(gi); }
                    x += 1; y += 1;
                }
                if y == pat.len() && !gis.is_empty() {
                    out.push(gis);
                    if out.len() >= 3000 { return out; }
                    i = x.max(i + 1);
                } else { i += 1; }
            }
        }
        out
    }

    /// 찾은 곳이 원고의 한 구간으로 이어지면 그 구간.
    fn hit_range(&self, gis: &[usize]) -> Option<(FileId, usize, usize)> {
        let f = self.glyph(*gis.first()?).file?;
        let (mut s, mut e) = (usize::MAX, 0);
        for &gi in gis {
            let g = self.glyph(gi);
            if !editable(g) || g.file != Some(f) { return None; }
            s = s.min(g.s as usize); e = e.max(g.e as usize);
        }
        (e > s).then_some((f, s, e))
    }

    /// 찾은 곳의 원고 글이 지면의 글과 글자 그대로 같은지(사이에 꾸밈 표식이나 다른 요소가 끼지 않았는지).
    fn hit_clean(&self, gis: &[usize], file: FileId, s: usize, e: usize) -> bool {
        let Some(text) = self.world.text_of(file) else { return false };
        let Some(src) = text.get(s..e) else { return false };
        let strip = |t: &str| t.chars().filter(|c| !c.is_whitespace() && *c != '\\').collect::<String>();
        let page: String = gis.iter().map(|&gi| self.ix.text_of(gi)).collect();
        strip(src) == strip(&page)
    }

    fn hit_rects(&self, gis: &[usize]) -> Vec<Value> {
        let mut rects = vec![];
        let mut cur: Option<(u32, u32, f32, f32, f32, f32)> = None; // page, line, x0, x1, y, size
        for &gi in gis {
            let g = self.glyph(gi);
            match &mut cur {
                Some(c) if c.0 == g.page && c.1 == g.line => { c.2 = c.2.min(g.x0); c.3 = c.3.max(g.x1); c.5 = c.5.max(g.size); }
                _ => {
                    if let Some(c) = cur.take() { rects.push(json!([c.0, c.2, c.4 - c.5 * 0.88, c.3 - c.2, c.5 * 1.2])); }
                    cur = Some((g.page, g.line, g.x0, g.x1, g.y, g.size));
                }
            }
        }
        if let Some(c) = cur { rects.push(json!([c.0, c.2, c.4 - c.5 * 0.88, c.3 - c.2, c.5 * 1.2])); }
        rects
    }

    /// 화면의 찾기 창에 보낼 결과.
    pub fn find(&self, q: &str, case: bool) -> Value {
        let hits = self.hits(q, case);
        let list: Vec<Value> = hits.iter().map(|h| json!({
            "page": self.glyph(h[0]).page,
            "rects": self.hit_rects(h),
            "ok": self.hit_range(h).is_some(),
        })).collect();
        json!({ "hits": list, "rev": self.rev })
    }

    /// `i`번째 찾은 곳을 선택한다. 지면에서 고칠 수 없는 글이면 선택하지 않는다.
    fn find_select(&mut self, q: &str, case: bool, i: usize) -> bool {
        let hits = self.hits(q, case);
        let Some(h) = hits.get(i) else { return false };
        match self.hit_range(h) {
            Some((f, s, e)) => {
                let g = self.glyph(h[0]);
                // 같은 원고 글이 여러 곳에 찍히는 경우(템플릿)에도 찾은 그 자리에 커서가 놓이도록
                self.caret = Some(Caret { pos: Pos { file: f, off: s }, aff_end: false, gi: Some(h[0]), page: g.page, x: g.x0, y: g.y, size: g.size });
                self.select(f, s, e);
                true
            }
            None => {
                self.caret = None; self.anchor = None;
                self.msg = Some(("찾은 글이 템플릿이 만든 글이거나 여러 요소에 걸쳐 있어, 지면에서 바로 바꿀 수 없습니다.".into(), "info"));
                false
            }
        }
    }

    fn replace_one(&mut self, q: &str, case: bool, i: usize, with: &str) {
        if !self.find_select(q, case, i) { return; }
        self.apply(0, true, |w| edit::insert(w, with));
    }

    fn replace_all(&mut self, q: &str, case: bool, with: &str) {
        let hits = self.hits(q, case);
        let mut by_file: Vec<(FileId, Vec<(usize, usize)>)> = vec![];
        let mut skipped = 0usize;
        for h in &hits {
            match self.hit_range(h) {
                Some((f, s, e)) if self.hit_clean(h, f, s, e) => {
                    match by_file.iter_mut().find(|x| x.0 == f) { Some(x) => x.1.push((s, e)), None => by_file.push((f, vec![(s, e)])) }
                }
                _ => skipped += 1,
            }
        }
        let mut done = 0usize;
        for (file, mut rs) in by_file {
            rs.sort(); rs.dedup();
            let Some(before) = self.world.text_of(file) else { continue };
            let mut w = Work::new(before.clone(), 0, 0);
            let (mut n, mut floor) = (0usize, usize::MAX);
            for &(s, e) in rs.iter().rev() {
                if e > floor { continue; }
                let mut t = w.clone();
                t.anchor = s; t.caret = e;
                if edit::insert(&mut t, with).is_ok() { w = t; n += 1; floor = s; } else { skipped += 1; }
            }
            if n == 0 { continue; }
            match self.try_text(file, &w.text) {
                Ok(()) => {
                    self.push_undo(Step { file, before, after: w.text, caret_before: None, caret_after: None, kind: 0, time: Instant::now() });
                    done += n;
                }
                Err(_) => skipped += n,
            }
        }
        if done > 0 { self.caret = None; self.anchor = None; }
        let mut m = crate::lang::f("{0}곳을 바꿨습니다.", &[&done]);
        if skipped > 0 { m.push(' '); m.push_str(&crate::lang::f("{0}곳은 지면에서 바로 바꿀 수 없어 건너뛰었습니다.", &[&skipped])); }
        self.msg = Some((m, if done == 0 && skipped > 0 { "warn" } else { "info" }));
    }

    fn sel_range(&self) -> Option<(usize, usize)> {
        let c = self.caret?; let a = self.anchor?;
        if a.file != c.pos.file || a.off == c.pos.off { return None; }
        Some((a.off.min(c.pos.off), a.off.max(c.pos.off)))
    }

    // ───────────── 요소(통째로 지우기·옮기기)

    fn pick(&mut self, page: usize, x: f32, y: f32) {
        self.caret = None; self.anchor = None; self.lock = None; self.elem = None;
        let Some((gi, _)) = self.hit_any(page, x, y) else { return };
        // 이 글자가 원고 어디에서 왔는지: 모르면 같은 줄의 가까운 글자를 따른다
        let mut at: Option<(FileId, usize)> = None;
        let line = self.glyph(gi).line;
        for d in 0..80usize {
            for i in [gi.checked_sub(d), gi.checked_add(d)].into_iter().flatten() {
                if let Some(g) = self.ix.glyphs.get(i) {
                    if g.line != line && d > 0 { continue; }
                    if let Some(f) = g.file { if self.world.in_project(f) { at = Some((f, g.s as usize)); break; } }
                }
            }
            if at.is_some() { break; }
        }
        let Some((file, pos)) = at else {
            self.msg = Some(("이 글자가 원고의 어느 요소에서 왔는지 찾지 못했습니다.".into(), "warn"));
            return;
        };
        let text = self.world.text_of(file).unwrap_or_default();
        match edit::elem_chain(&text, pos).into_iter().next() {
            Some(e) => {
                self.elem = Some(ElemSel { file, range: e.range, name: e.name, at: (page as u32, x, y), bx: None });
                self.refresh_box(None);
            }
            None => self.msg = Some(("이 글은 따로 떼어 낼 요소가 아닙니다. 글자를 끌어서 선택해 지워 주세요.".into(), "info")),
        }
    }

    // ───────────── 상자·표의 너비

    fn page_boxes(&self, page: usize) -> Vec<Bx> {
        let (Some(p), Some(&(g, _))) = (self.doc.pages().get(page), self.ix.page_ranges.get(page)) else { return vec![] };
        let mut w = BoxWalk { world: &self.world, page: page as u32, g, out: vec![], pending: None };
        w.frame(&p.frame, typst::layout::Transform::identity(), None, None, 0);
        w.out
    }

    /// 골라 둔 요소의 상자를 지면에서 찾는다.
    /// `near`: 방금 고친 상자가 있으면 그 원고 자리와 지면 자리(같은 상자를 다시 찾는다).
    fn find_box(&self, el: &ElemSel, near: Option<(FileId, std::ops::Range<usize>, f32, f32)>) -> Option<BoxSel> {
        let page = el.at.0 as usize;
        let boxes = self.page_boxes(page);
        let (a, b) = (el.range.start as u32, el.range.end as u32);
        let inside = |g: &G| g.file == Some(el.file) && g.s >= a && g.s < b;
        let src_of = |bx: &Bx| bx.grid.clone().or(bx.shape.clone());
        let (px, py) = match &near { Some(n) => (n.2 + 1.0, n.3 + 1.0), None => (el.at.1, el.at.2) };
        
        // 1) 이 요소가 바로 그 상자(표·격자, 또는 채움·테두리가 있는 상자)
        let want = match &near { Some(n) => (n.0, n.1.clone()), None => (el.file, el.range.clone()) };
        let mut found: Option<(&Bx, (FileId, std::ops::Range<usize>))> =
            boxes.iter().filter(|x| src_of(x).is_some_and(|s| s == want))
                .min_by(|p, q| p.dist(px, py).partial_cmp(&q.dist(px, py)).unwrap()).map(|x| (x, want.clone()));
        // 2) 누른 자리를 품고 이 요소의 글만 담은 상자들(바깥 것부터)
        if found.is_none() && near.is_none() {
            let n = self.ix.glyphs.len();
            let mut cands: Vec<&Bx> = boxes.iter().filter(|x| x.has(px, py)).filter(|x| {
                let (mut any, mut ok) = (false, true);
                for g in &self.ix.glyphs[x.g0.min(n)..x.g1.min(n)] {
                    if g.file == Some(el.file) { if inside(g) { any = true; } else { ok = false; break; } }
                }
                any && ok
            }).collect();
            cands.sort_by_key(|x| x.depth);
            let own = self.world.text_of(el.file).and_then(|t| edit::resize_kind(&t, &el.range));
            if matches!(own, Some((_, false))) {
                // 이 요소가 바로 상자(채움·테두리가 없어 원고 자리가 지면에 남지 않은 것): 가장 바깥 묶음이 그 상자다
                found = cands.first().map(|x| (*x, (el.file, el.range.clone())));
            } else {
                // 이 요소가 그려 낸 상자(템플릿 함수가 만든 상자·표): 원고 자리를 아는 가장 바깥 것
                for x in cands {
                    let Some(s) = src_of(x) else { continue };
                    if self.world.text_of(s.0).is_some_and(|t| edit::resize_kind(&t, &s.1).is_some()) {
                        found = Some((x, s));
                        break;
                    }
                }
            }
        }
        let (bx, (file, range)) = found?;
        let text = self.world.text_of(file)?;
        let (name, grid) = edit::resize_kind(&text, &range)?;
        let cols = if grid { bx.col_widths() } else { vec![] };
        let bounds = if cols.is_empty() { vec![] } else { (1..bx.lefts.len()).map(|i| bx.lefts[i] - bx.gutter / 2.0).collect() };
        // 같은 원고 자리에서 나온 상자가 책 전체에 몇 곳인지(템플릿 요소)
        let mut copies = 0;
        for p in 0..self.doc.pages().len() {
            let v = if p == page { None } else { Some(self.page_boxes(p)) };
            copies += v.as_ref().unwrap_or(&boxes).iter().filter(|x| src_of(x).is_some_and(|s| s.0 == file && s.1 == range)).count();
            if copies > 999 { break; }
        }
        Some(BoxSel { file, range, name, grid, page: bx.page, x: bx.x, y: bx.y, w: bx.w, h: bx.h, cols, bounds, copies: copies.max(1) })
    }

    fn refresh_box(&mut self, near: Option<(FileId, std::ops::Range<usize>, f32, f32)>) {
        let Some(el) = self.elem.take() else { return };
        let bx = self.find_box(&el, near);
        self.elem = Some(ElemSel { bx, ..el });
    }

    /// 상자의 오른쪽 가장자리(`col` 없음)나 칸 사이 경계(`col`번째 칸 뒤)를 `dx`pt 만큼 끈다.
    fn elem_resize(&mut self, col: Option<usize>, dx: f32) {
        let Some(mut el) = self.elem.take() else { return };
        let Some(bx) = el.bx.take() else {
            self.msg = Some(("이 요소는 지면에서 너비를 바꿀 수 없습니다.".into(), "warn"));
            self.elem = Some(el);
            return;
        };
        let Some(before) = self.world.text_of(bx.file) else { self.elem = Some(ElemSel { bx: Some(bx), ..el }); return };
        match edit::elem_resize(&before, &bx.range, col, dx, Some(bx.w), &bx.cols) {
            Ok(t) => {
                if self.apply_file(bx.file, t.clone()) {
                    // 고친 자리 뒤의 원고 자리는 그만큼 밀린다
                    let (ob, nb) = (before.as_bytes(), t.as_bytes());
                    let mut pre = 0;
                    while pre < ob.len() && pre < nb.len() && ob[pre] == nb[pre] { pre += 1; }
                    let mut suf = 0;
                    while suf < ob.len() - pre && suf < nb.len() - pre && ob[ob.len() - 1 - suf] == nb[nb.len() - 1 - suf] { suf += 1; }
                    let map = |p: usize| if p <= pre { p } else if p >= ob.len() - suf { p + nb.len() - ob.len() } else { pre };
                    let range = map(bx.range.start)..map(bx.range.end);
                    if el.file == bx.file { el.range = map(el.range.start)..map(el.range.end); }
                    self.elem = Some(el);
                    self.refresh_box(Some((bx.file, range, bx.x, bx.y)));
                    if self.elem.as_ref().is_some_and(|e| e.bx.is_none()) { self.refresh_box(None); }
                } else { self.elem = Some(ElemSel { bx: Some(bx), ..el }); }
            }
            Err(e) => { self.msg = Some((e, "warn")); self.elem = Some(ElemSel { bx: Some(bx), ..el }); }
        }
    }

    fn hit_any(&self, page: usize, x: f32, y: f32) -> Option<(usize, bool)> {
        let &(a, b) = self.ix.page_ranges.get(page)?;
        let mut best: Option<(usize, f32)> = None;
        for gi in a..b {
            let g = &self.ix.glyphs[gi];
            let (top, bot) = (g.y - g.size * 0.92, g.y + g.size * 0.36);
            let dy = if y < top { top - y } else if y > bot { y - bot } else { 0.0 };
            let dx = if x < g.x0 { g.x0 - x } else if x > g.x1 { x - g.x1 } else { 0.0 };
            let sc = dy * 6.0 + dx;
            if sc < 60.0 && best.map_or(true, |b| sc < b.1) { best = Some((gi, sc)); }
        }
        best.map(|(gi, sc)| (gi, sc == 0.0))
    }

    fn elem_grow(&mut self) {
        let Some(el) = &self.elem else { return };
        let text = self.world.text_of(el.file).unwrap_or_default();
        let chain = edit::elem_chain(&text, el.range.start);
        let next = chain.into_iter().find(|e| e.range.start <= el.range.start && e.range.end >= el.range.end && e.range != el.range);
        match next {
            Some(e) => {
                let (file, at) = (el.file, el.at);
                self.elem = Some(ElemSel { file, range: e.range, name: e.name, at, bx: None });
                self.refresh_box(None);
            }
            None => self.msg = Some(("더 넓힐 수 없습니다.".into(), "info")),
        }
    }

    fn apply_file(&mut self, file: FileId, new_text: String) -> bool {
        let Some(before) = self.world.text_of(file) else { return false };
        let w = Work::new(new_text, 0, 0);
        if !self.commit(file, &before, &w) { return false; }
        self.push_undo(Step { file, before, after: w.text, caret_before: None, caret_after: None, kind: 0, time: Instant::now() });
        true
    }

    fn elem_delete(&mut self) {
        let Some(el) = self.elem.take() else { return };
        let text = self.world.text_of(el.file).unwrap_or_default();
        match edit::elem_delete(&text, &el.range) {
            Ok(t) => { if !self.apply_file(el.file, t) { self.elem = Some(el); } }
            Err(e) => { self.msg = Some((e, "warn")); self.elem = Some(el); }
        }
    }

    fn elem_move(&mut self, dx: f32, dy: f32) {
        let Some(el) = self.elem.take() else { return };
        let text = self.world.text_of(el.file).unwrap_or_default();
        match edit::elem_move(&text, &el.range, dx, dy) {
            Ok((t, r)) => {
                if self.apply_file(el.file, t.clone()) {
                    let name = edit::elem_chain(&t, r.start).into_iter().find(|e| e.range == r).map(|e| e.name).unwrap_or(el.name.clone());
                    self.elem = Some(ElemSel { file: el.file, range: r, name, at: (el.at.0, el.at.1 + dx, el.at.2 + dy), bx: None });
                    self.refresh_box(None);
                } else { self.elem = Some(el); }
            }
            Err(e) => { self.msg = Some((e, "warn")); self.elem = Some(el); }
        }
    }

    fn elem_view(&self) -> Value {
        let Some(el) = &self.elem else { return Value::Null };
        let (a, b) = (el.range.start as u32, el.range.end as u32);
        let inside = |g: &G| g.file == Some(el.file) && g.s >= a && g.s < b;
        let n = self.ix.glyphs.len();
        let mut rects: Vec<Value> = vec![];
        let mut copies: std::collections::HashMap<u32, u32> = Default::default();
        let mut i = 0;
        while i < n {
            if !inside(&self.ix.glyphs[i]) { i += 1; continue; }
            // 한 덩어리: 사이에 낀(원고 자리를 모르는) 글자 몇 개는 건너뛴다
            let (mut j, mut last) = (i, i);
            while j < n && j - last <= 8 && self.ix.glyphs[j].page == self.ix.glyphs[i].page {
                if inside(&self.ix.glyphs[j]) { last = j; }
                j += 1;
            }
            // 같은 줄에서 바로 뒤따르는, 자리를 모르는 글자(세어 넣은 숫자 등)까지 품는다
            let mut end = last;
            while end + 1 < n && self.ix.glyphs[end + 1].line == self.ix.glyphs[last].line && self.ix.glyphs[end + 1].file.is_none() { end += 1; }
            let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for g in &self.ix.glyphs[i..=end] {
                x0 = x0.min(g.x0); x1 = x1.max(g.x1);
                y0 = y0.min(g.y - g.size * 0.92); y1 = y1.max(g.y + g.size * 0.3);
                if inside(g) && charlevel(g) && g.e > g.s { *copies.entry(g.s).or_default() += 1; }
            }
            rects.push(json!([self.ix.glyphs[i].page, x0 - 2.0, y0 - 2.0, x1 - x0 + 4.0, y1 - y0 + 4.0]));
            if rects.len() >= 400 { break; }
            i = end + 1;
        }
        let line = self.world.source_of(el.file).and_then(|s| s.lines().byte_to_line(el.range.start)).unwrap_or(0) + 1;
        json!({
            "rects": rects,
            "copies": copies.values().copied().max().unwrap_or(0),
            "name": el.name,
            "file": el.file.vpath().get_without_slash(),
            "line": line,
            "box": el.bx.as_ref().map(|b| json!({
                "page": b.page, "x": b.x, "y": b.y, "w": b.w, "h": b.h,
                "bounds": b.bounds, "cols": b.cols, "grid": b.grid, "name": b.name, "copies": b.copies,
                "file": b.file.vpath().get_without_slash(),
                "line": self.world.source_of(b.file).and_then(|s| s.lines().byte_to_line(b.range.start)).unwrap_or(0) + 1,
                "own": b.file == el.file && b.range == el.range,
            })),
        })
    }

    // ───────────── 고치기

    fn flush_comp(&mut self) {
        if let Some(cp) = self.comp.take() {
            if let Some(now) = self.world.text_of(cp.file) {
                if now != cp.base_text {
                    self.push_undo(Step { file: cp.file, before: cp.base_text, after: now, caret_before: cp.caret_before, caret_after: self.caret.map(|c| c.pos), kind: 0, time: Instant::now() });
                }
            }
        }
    }

    fn push_undo(&mut self, st: Step) {
        self.redo.clear();
        if st.kind != 0 {
            if let Some(last) = self.undo.last_mut() {
                if last.kind == st.kind && last.file == st.file && last.caret_after == st.caret_before && st.time.duration_since(last.time).as_millis() < 1500 {
                    last.after = st.after; last.caret_after = st.caret_after; last.time = st.time;
                    return;
                }
            }
        }
        self.undo.push(st);
        if self.undo.len() > 500 { self.undo.remove(0); }
    }

    fn work(&self) -> Option<(FileId, Work)> {
        let c = self.caret?;
        let text = self.world.text_of(c.pos.file)?;
        let anchor = match self.anchor { Some(a) if a.file == c.pos.file => a.off, _ => c.pos.off };
        Some((c.pos.file, Work::new(text, c.pos.off, anchor)))
    }

    /// 연산을 적용하고 다시 조판한다. 조판이 깨지면 되돌린다.
    fn apply(&mut self, kind: u8, aff_end: bool, f: impl FnOnce(&mut Work) -> edit::R<()>) -> bool {
        let Some((file, mut w)) = self.work() else {
            self.msg = Some(("먼저 고칠 자리를 눌러 커서를 놓아 주세요.".into(), "info"));
            return false;
        };
        let before = w.text.clone();
        let caret_before = self.caret.map(|c| c.pos);
        if let Err(e) = f(&mut w) { self.msg = Some((e, "warn")); return false; }
        if !self.commit(file, &before, &w) { return false; }
        let hint = self.hint();
        self.caret = Some(self.locate(Pos { file, off: w.caret }, aff_end, hint));
        self.anchor = if w.anchor != w.caret { Some(Pos { file, off: w.anchor }) } else { None };
        if w.text != before {
            self.push_undo(Step { file, before, after: w.text, caret_before, caret_after: self.caret.map(|c| c.pos), kind, time: Instant::now() });
        }
        if let Some(name) = &w.note { self.note_killed(name); }
        true
    }

    /// 원고 범위 [a, b)의 글자가 지면에 굵게 찍혔는지: (굵은 글자 수, 굵기를 알 수 있는 글자 수).
    fn bold_count(&self, file: FileId, a: usize, b: usize) -> (usize, usize) {
        let Some(v) = self.ix.by_file.get(&file) else { return (0, 0) };
        let from = v.partition_point(|&(s, _)| (s as usize) < a);
        let (mut nb, mut n) = (0, 0);
        for &(s, gi) in &v[from..] {
            if s as usize >= b { break; }
            match self.ix.glyphs[gi as usize].bold { 1 => { nb += 1; n += 1; } 0 => n += 1, _ => {} }
        }
        (nb, n)
    }

    /// 굵게 켜고 끄기. 원고의 표식만 보지 않고 지면에 실제로 찍힌 굵기를 따른다:
    /// `#text(weight: "bold")`나 템플릿·show 규칙이 굵게 만든 글자도 굵은 글자로 보고 푼다.
    fn toggle_bold(&mut self) {
        let Some((file, mut w)) = self.work() else {
            self.msg = Some(("먼저 고칠 자리를 눌러 커서를 놓아 주세요.".into(), "info"));
            return;
        };
        let before = w.text.clone();
        let caret_before = self.caret.map(|c| c.pos);
        let Some((a, b)) = w.sel() else {
            self.msg = Some(("먼저 서식을 바꿀 글자를 끌어서 선택해 주세요.".into(), "warn"));
            return;
        };
        let (nb, n) = self.bold_count(file, a, b);
        let on = edit::fmt_state(&w.text, a, b)[0] || (n > 0 && nb == n);
        // 1) 굵기를 만든 꾸밈을 원고에서 걷어 낸다
        if let Err(e) = edit::bold_strip(&mut w, on) { self.msg = Some((e, "warn")); return; }
        let mut more = w.text == before;
        if !more {
            if !self.commit(file, &before, &w) { return; }
            // 걷어 냈는데도 지면의 굵기가 그대로이면(바깥 꾸밈이나 템플릿이 준 굵기) 덧씌워야 한다
            if let Some((a, b)) = w.sel() {
                let (nb, n) = self.bold_count(file, a, b);
                more = if on { nb > 0 } else { n > 0 && nb < n };
            }
        }
        // 2) 덧씌운다: 끌 때는 `#text(weight: "regular")[..]`, 켤 때는 `*..*`
        if more {
            let mut w2 = w.clone();
            match edit::bold_wrap(&mut w2, on) {
                Ok(()) => {
                    let mid = w.text.clone();
                    if self.commit(file, &mid, &w2) { w = w2; } else if mid == before { return; }
                }
                Err(e) => if w.text == before { self.msg = Some((e, "warn")); return; },
            }
        }
        let hint = self.hint();
        self.caret = Some(self.locate(Pos { file, off: w.caret }, true, hint));
        self.anchor = if w.anchor != w.caret { Some(Pos { file, off: w.anchor }) } else { None };
        if w.text != before {
            self.push_undo(Step { file, before, after: w.text, caret_before, caret_after: self.caret.map(|c| c.pos), kind: 0, time: Instant::now() });
        }
    }

    fn note_killed(&mut self, name: &str) {
        self.msg = Some((crate::lang::f("#{0} 요소를 통째로 지웠습니다. 되돌리려면 Ctrl+Z를 누르세요.", &[&name]), "info"));
    }

    /// 파일 본문을 바꿔 조판해 본다. 조판이 깨지면 본문을 되돌리고 오류를 돌려준다
    /// (지면과 색인은 앞선 조판 그대로이므로 다시 조판할 필요가 없다).
    fn try_text(&mut self, file: FileId, text: &str) -> Result<(), String> {
        let Some(before) = self.world.text_of(file) else { return Err(String::new()) };
        if before == text { return Ok(()); }
        self.set_text(file, text);
        match self.compile() {
            Ok(()) => Ok(()),
            Err(e) => { self.set_text(file, &before); Err(e) }
        }
    }

    fn commit(&mut self, file: FileId, before: &str, w: &Work) -> bool {
        if w.text == before { return true; }
        if let Err(e) = self.try_text(file, &w.text) {
            self.msg = Some((format!("이 수정은 조판 오류가 나서 적용하지 않았습니다. ({e})"), "error"));
            return false;
        }
        true
    }

    /// 지면에서 옆 글로 커서를 옮긴다(글 상자의 가장자리처럼 더 지울 것이 없는 자리에서). 자리가 바뀌면 true.
    fn hop(&mut self, fwd: bool) -> bool {
        let Some(c) = self.caret else { return false };
        let Some(gi) = c.gi else { return false };
        let g = self.glyph(gi);
        let mut new: Option<Caret> = None;
        // 커서가 이 글자에서 떨어진 자리(요소 뒤 등)에 있었으면 먼저 이 글자 곁으로
        if editable(g) && g.file == Some(c.pos.file) {
            if !fwd && c.pos.off > g.e as usize { new = Some(self.caret_on(gi, g.e as usize)); }
            if fwd && c.pos.off < g.s as usize { new = Some(self.caret_on(gi, g.s as usize)); }
        }
        let mut i = gi;
        for _ in 0..6 {
            if new.is_some_and(|n| n.pos != c.pos) { break; }
            let Some(n) = self.next_editable(i, fwd) else { return false };
            let ng = self.glyph(n);
            new = Some(self.caret_on(n, if fwd { ng.s } else { ng.e } as usize));
            i = n;
        }
        match new {
            Some(n) if n.pos != c.pos => { self.caret = Some(n); self.anchor = None; true }
            _ => false,
        }
    }

    /// Backspace·Delete를 `n`번(낱말 단위 포함). 글줄에 낀 꾸밈은 그냥 넘어가며 지우고,
    /// 글 상자의 가장자리에 닿으면 커서를 지면의 옆 글로 옮겨, 누르고 있기만 해도 이어서 지워지게 한다.
    fn del_many(&mut self, fwd: bool, word: bool, n: usize) {
        let op = |w: &mut Work| -> edit::R<()> {
            w.edge = false; w.note = None;
            if word { edit::del_word(w, fwd) } else if fwd { edit::delete(w) } else { edit::backspace(w) }
        };
        let mut left = n.clamp(1, 80);
        let mut killed: Option<String> = None;
        let mut free = 0;
        while left > 0 {
            let Some((file, w0)) = self.work() else {
                self.msg = Some(("먼저 고칠 자리를 눌러 커서를 놓아 주세요.".into(), "info"));
                return;
            };
            let caret_before = self.caret.map(|c| c.pos);
            // 1) 조판 없이 이어서 지울 수 있는 데까지 지운다
            let mut trail: Vec<Work> = vec![];
            let (mut edge, mut err) = (false, None::<String>);
            while trail.len() < left {
                let prev = trail.last().unwrap_or(&w0);
                let mut t = prev.clone();
                match op(&mut t) {
                    Err(e) => { err = Some(e); break; }
                    Ok(()) if t.edge || t.text == prev.text => { edge = true; break; }
                    Ok(()) => trail.push(t),
                }
            }
            // 2) 조판한다. 깨지면: 한두 걸음 더 지워 보고(빈 껍데기가 조판을 깨는 경우), 그래도 안 되면 한 걸음씩
            let l0 = trail.len();
            let mut ok_len = 0;
            let mut fail: Option<String> = None;
            if l0 > 0 {
                let mut cands = vec![l0, l0 + 1, l0 + 2];
                if l0 > 1 { cands.extend([1, 2, 3]); }
                let mut tried: Vec<usize> = vec![];
                for k in cands {
                    if tried.contains(&k) { continue; }
                    tried.push(k);
                    while trail.len() < k {
                        let prev = trail.last().unwrap();
                        let mut t = prev.clone();
                        match op(&mut t) {
                            Ok(()) if !t.edge && t.text != prev.text => trail.push(t),
                            _ => break,
                        }
                    }
                    if trail.len() < k { continue; }
                    match self.try_text(file, &trail[k - 1].text) {
                        Ok(()) => { ok_len = k; break; }
                        Err(e) => { fail.get_or_insert(e); }
                    }
                }
                if ok_len == 0 {
                    // 어떻게 지워도 조판이 깨지는 글자: 알리고 건너뛴다
                    self.msg = Some((format!("이 글자는 지우면 조판 오류가 나서 건너뛰었습니다. ({})", fail.unwrap_or_default()), "error"));
                    self.hop(fwd);
                    return;
                }
                let w = trail[ok_len - 1].clone();
                for t in &trail[..ok_len] { if let Some(nm) = &t.note { killed = Some(nm.clone()); } }
                let hint = self.hint();
                self.caret = Some(self.locate(Pos { file, off: w.caret }, !fwd, hint));
                self.anchor = None;
                self.push_undo(Step { file, before: w0.text, after: w.text, caret_before, caret_after: self.caret.map(|c| c.pos), kind: 2, time: Instant::now() });
                left = left.saturating_sub(ok_len.min(l0).max(1));
                if ok_len < l0 { continue; }
            }
            if let Some(e) = err { self.msg = Some((e, "warn")); return; }
            if edge {
                let spot = |c: Option<Caret>| c.map(|c| (c.page, c.x, c.y));
                let from = spot(self.caret);
                if !self.hop(fwd) { break; }
                // 보이는 자리가 그대로면(빈 글 상자에서 바로 곁의 글자로) 이 누름으로 이어서 지운다
                let same = matches!((from, spot(self.caret)), (Some(a), Some(b)) if a.0 == b.0 && (a.1 - b.1).abs() < 0.5 && (a.2 - b.2).abs() < 0.5);
                if same && free < 3 { free += 1; } else { left = left.saturating_sub(1); }
            } else if l0 == 0 { break; }
        }
        if let Some(name) = killed { self.note_killed(&name); }
    }

    fn compose(&mut self, text: &str, end: bool) {
        if self.comp.is_none() {
            let Some((file, w)) = self.work() else { return };
            self.comp = Some(Comp { file, base_text: w.text, base_caret: w.caret, base_anchor: w.anchor, caret_before: self.caret.map(|c| c.pos) });
        }
        let cp = self.comp.as_ref().unwrap();
        let file = cp.file;
        let mut w = Work::new(cp.base_text.clone(), cp.base_caret, cp.base_anchor);
        let cur = self.world.text_of(file).unwrap_or_default();
        let ok = match edit::insert(&mut w, text) {
            Ok(()) => self.commit(file, &cur, &w),
            Err(e) => { self.msg = Some((e, "warn")); false }
        };
        if ok {
            let hint = self.hint();
            self.caret = Some(self.locate(Pos { file, off: w.caret }, true, hint));
            self.anchor = None;
        }
        if end || !ok { self.flush_comp(); }
    }

    fn tab_width(&self) -> f32 {
        let Some(c) = self.caret else { return 20.0 };
        let stop = (c.size * 2.0).max(8.0);
        let left = c.gi.map(|gi| { let (a, b) = self.line_span(gi); (a..=b).map(|i| self.glyph(i).x0).fold(f32::MAX, f32::min) }).unwrap_or(c.x);
        let rel = (c.x - left).max(0.0);
        let mut w = stop - rel % stop;
        if w < c.size * 0.3 { w += stop; }
        w
    }

    fn undo_redo(&mut self, undo: bool) {
        let st = if undo { self.undo.pop() } else { self.redo.pop() };
        let Some(st) = st else { return };
        let (text, pos) = if undo { (&st.before, st.caret_before) } else { (&st.after, st.caret_after) };
        self.set_text(st.file, text);
        if let Err(e) = self.compile() { self.msg = Some((format!("되돌리는 중 조판 오류: {e}"), "error")); }
        let hint = self.hint();
        self.caret = pos.map(|p| self.locate(p, true, hint));
        self.anchor = None;
        if undo { self.redo.push(st); } else { self.undo.push(st); }
    }

    pub fn save(&mut self) -> bool {
        self.flush_comp();
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        let name: String = self.title.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) { '_' } else { c }).collect();
        let dir = crate::data_dir().join("backup").join(name).join(stamp);
        match self.world.save_all(&dir) {
            Ok(names) if names.is_empty() => { self.msg = Some(("바뀐 내용이 없습니다.".into(), "info")); true }
            Ok(names) => { self.msg = Some((format!("저장했습니다: {}", names.join(", ")), "info")); true }
            Err(e) => { self.msg = Some((e, "error")); false }
        }
    }

    pub fn act(&mut self, req: &Value) {
        let t0 = Instant::now();
        self.act_inner(req);
        self.last_ms = t0.elapsed().as_millis() as u64;
    }

    fn act_inner(&mut self, req: &Value) {
        self.msg = None;
        let op = req["op"].as_str().unwrap_or("");
        if !matches!(op, "compose" | "composeEnd") { self.flush_comp(); }
        if op != "click" && op != "move" { self.lock = None; self.goal_x = None; }
        if !op.starts_with("elem") && op != "pick" && op != "save" { self.elem = None; }
        let text = req["text"].as_str().unwrap_or("");
        match op {
            "click" => self.click(
                req["page"].as_u64().unwrap_or(0) as usize,
                req["x"].as_f64().unwrap_or(0.0) as f32, req["y"].as_f64().unwrap_or(0.0) as f32,
                req["extend"].as_bool().unwrap_or(false), req["dbl"].as_bool().unwrap_or(false), req["tpl"].as_bool().unwrap_or(false)),
            "move" => self.mv(req["dir"].as_str().unwrap_or(""), req["shift"].as_bool().unwrap_or(false)),
            "insert" => {
                let typing = !text.chars().any(|c| c.is_whitespace()) && self.sel_range().is_none();
                self.apply(typing as u8, true, |w| edit::insert(w, text));
            }
            "compose" => self.compose(text, false),
            "composeEnd" => self.compose(text, true),
            "backspace" | "delete" | "wordBack" | "wordDel" => {
                let n = req["n"].as_u64().unwrap_or(1) as usize;
                self.del_many(matches!(op, "delete" | "wordDel"), matches!(op, "wordBack" | "wordDel"), n);
            }
            "enter" => { self.apply(0, false, edit::enter); }
            "linebreak" => { self.apply(0, false, edit::linebreak); }
            "tab" => { let wd = self.tab_width(); self.apply(0, true, |w| edit::tab(w, wd)); }
            "bold" => self.toggle_bold(),
            "italic" | "underline" | "strike" => {
                let st = match op { "italic" => Style::Italic, "underline" => Style::Under, _ => Style::Strike };
                let u = self.under_fn.clone();
                self.apply(0, true, |w| edit::toggle(w, st, &u));
            }
            "indent" => { self.apply(0, true, |w| edit::hanging(w, 1)); }
            "outdent" => { self.apply(0, true, |w| edit::hanging(w, -1)); }
            "align" => {
                let how = req["how"].as_str().filter(|h| ["left", "center", "right"].contains(h)).map(|h| h.to_string());
                self.apply(0, true, |w| edit::align(w, how.as_deref()));
            }
            "selectAll" => self.select_all(),
            "findSel" | "replace" | "replaceAll" => {
                let q = req["q"].as_str().unwrap_or("");
                let case = req["case"].as_bool().unwrap_or(false);
                let i = req["i"].as_u64().unwrap_or(0) as usize;
                let with = req["with"].as_str().unwrap_or("");
                match op {
                    "findSel" => { self.find_select(q, case, i); }
                    "replace" => self.replace_one(q, case, i, with),
                    _ => self.replace_all(q, case, with),
                }
            }
            "undo" => self.undo_redo(true),
            "redo" => self.undo_redo(false),
            "save" => { self.save(); }
            "pick" => self.pick(req["page"].as_u64().unwrap_or(0) as usize, req["x"].as_f64().unwrap_or(0.0) as f32, req["y"].as_f64().unwrap_or(0.0) as f32),
            "elemGrow" => self.elem_grow(),
            "elemDelete" => self.elem_delete(),
            "elemMove" => self.elem_move(req["dx"].as_f64().unwrap_or(0.0) as f32, req["dy"].as_f64().unwrap_or(0.0) as f32),
            "elemResize" => self.elem_resize(req["col"].as_u64().map(|c| c as usize), req["dx"].as_f64().unwrap_or(0.0) as f32),
            "elemClear" => { self.elem = None; }
            _ => {}
        }
    }

    // ───────────── 화면에 보낼 정보

    fn sel_view(&self) -> (Vec<Value>, String) {
        let Some((a, b)) = self.sel_range() else { return (vec![], String::new()) };
        let file = self.caret.unwrap().pos.file;
        let Some(v) = self.ix.by_file.get(&file) else { return (vec![], String::new()) };
        let i0 = v.partition_point(|x| (x.0 as usize) < a.saturating_sub(64));
        let mut gis: Vec<usize> = vec![];
        for &(s, gi) in &v[i0..] {
            if s as usize >= b { break; }
            let g = self.glyph(gi as usize);
            if (g.e as usize) > a && g.e > g.s { gis.push(gi as usize); }
        }
        // 글
        let src = self.world.text_of(file).unwrap_or_default();
        let mut text = String::new();
        let mut last_e: Option<usize> = None;
        let mut seen_s = usize::MAX;
        for &gi in &gis {
            let g = self.glyph(gi);
            if g.s as usize == seen_s { continue; }
            seen_s = g.s as usize;
            if let Some(le) = last_e {
                if (g.s as usize) > le && src.get(le..g.s as usize).is_some_and(|t| t.chars().any(|c| c.is_whitespace())) && !text.ends_with(' ') { text.push(' '); }
            }
            text.push_str(self.ix.text_of(gi));
            last_e = Some(g.e as usize);
        }
        // 사각형: 화면 순서로 같은 줄끼리 묶는다
        gis.sort();
        let mut rects: Vec<Value> = vec![];
        let mut cur: Option<(u32, u32, f32, f32, f32, f32)> = None; // page, line, x0, x1, y, size
        for gi in gis {
            let g = self.glyph(gi);
            let (s, e) = (g.s as usize, g.e as usize);
            let fx = |off: usize| g.x0 + (g.x1 - g.x0) * (off.clamp(s, e) - s) as f32 / (e - s).max(1) as f32;
            let (x0, x1) = (fx(a), fx(b));
            match &mut cur {
                Some(c) if c.0 == g.page && c.1 == g.line && x0 <= c.3 + g.size * 1.2 && x0 >= c.2 - 0.5 => { c.3 = c.3.max(x1); c.5 = c.5.max(g.size); }
                _ => {
                    if let Some(c) = cur.take() { rects.push(json!([c.0, c.2, c.4 - c.5 * 0.88, c.3 - c.2, c.5 * 1.2])); }
                    cur = Some((g.page, g.line, x0, x1, g.y, g.size));
                }
            }
        }
        if let Some(c) = cur { rects.push(json!([c.0, c.2, c.4 - c.5 * 0.88, c.3 - c.2, c.5 * 1.2])); }
        (rects, text)
    }

    pub fn view(&mut self) -> Value {
        let pages: Vec<Value> = self.doc.pages().iter().zip(&self.hashes).map(|(p, h)| {
            json!([p.frame.width().to_pt(), p.frame.height().to_pt(), h])
        }).collect();
        let (sel, sel_text) = self.sel_view();
        let mut fmt = [false; 4];
        let mut align = None::<String>;
        let mut where_ = Value::Null;
        if let Some(c) = &self.caret {
            if let Some(src) = self.world.source_of(c.pos.file) {
                let (a, b) = self.sel_range().unwrap_or((c.pos.off, c.pos.off));
                fmt = edit::fmt_state(src.text(), a, b);
                // 굵게는 지면에 찍힌 굵기도 본다(템플릿이 굵게 만든 글자)
                let (nb, n) = if a < b { self.bold_count(c.pos.file, a, b) } else {
                    match c.gi.map(|gi| self.glyph(gi).bold) { Some(1) => (1, 1), Some(0) => (0, 1), _ => (0, 0) }
                };
                if n > 0 && nb == n { fmt[0] = true; }
                align = edit::align_state(src.text(), c.pos.off);
                let line = src.lines().byte_to_line(c.pos.off.min(src.text().len())).unwrap_or(0) + 1;
                where_ = json!({ "file": c.pos.file.vpath().get_without_slash(), "line": line });
            }
        }
        let dirty: Vec<String> = self.world.dirty_files().iter().map(|id| id.vpath().file_name().unwrap_or("").to_string()).collect();
        let msg = self.msg.clone();
        json!({
            "open": true,
            "title": self.title,
            "path": self.main_path.display().to_string(),
            "pages": pages,
            "caret": self.caret.map(|c| json!({ "page": c.page, "x": c.x, "y": c.y - c.size * 0.88, "h": c.size * 1.2 })),
            "lock": self.lock.as_ref().map(|l| json!({ "page": l.page, "x": l.x, "y": l.y, "w": l.w, "h": l.h, "reason": l.reason })),
            "sel": sel,
            "selText": sel_text,
            "fmt": { "bold": fmt[0], "italic": fmt[1], "underline": fmt[2], "strike": fmt[3], "align": align },
            "rev": self.rev,
            "dirty": dirty,
            "canUndo": !self.undo.is_empty() || self.comp.is_some(),
            "canRedo": !self.redo.is_empty(),
            "msg": msg.as_ref().map(|m| m.0.clone()),
            "msgKind": msg.as_ref().map(|m| m.1),
            "where": where_,
            "ms": self.last_ms,
            "timing": self.timing,
            "problems": [],
            "elem": self.elem_view(),
        })
    }

    pub fn page_svg(&self, i: usize) -> Option<String> {
        self.doc.pages().get(i).map(|p| typst_svg::svg(p, &Default::default()))
    }

    pub fn source_view(&self) -> Value {
        let Some(c) = &self.caret else { return json!({ "file": "", "text": "", "line": 0, "rev": self.rev }) };
        let Some(src) = self.world.source_of(c.pos.file) else { return json!({ "file": "", "text": "", "line": 0, "rev": self.rev }) };
        let line = src.lines().byte_to_line(c.pos.off.min(src.text().len())).unwrap_or(0) + 1;
        json!({ "file": c.pos.file.vpath().get_without_slash(), "text": src.text(), "line": line, "rev": self.rev })
    }
}

impl G {
    /// 낱말 선택이 글자 노드 밖으로 나가지 않도록 하는 아래 한계.
    fn node_start(&self, _text: &str) -> usize { 0 }
}
