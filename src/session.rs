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

struct ElemSel { file: FileId, range: std::ops::Range<usize>, name: String }

struct Lock { page: u32, x: f32, y: f32, w: f32, h: f32, reason: String }

struct Step {
    file: FileId,
    before: String,
    after: String,
    caret_before: Option<Pos>,
    caret_after: Option<Pos>,
    typing: bool,
    time: Instant,
}

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
            let cands: Vec<usize> = v[i0.saturating_sub(4)..i0].iter().map(|x| x.1 as usize).collect();
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

    fn click(&mut self, page: usize, x: f32, y: f32, extend: bool, dbl: bool) {
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

    fn mv(&mut self, dir: &str, shift: bool) {
        let Some(c) = self.caret else { return };
        let sel = self.sel_range();
        let new = match dir {
            "left" | "wordleft" => {
                if !shift && sel.is_some() { let (a, _) = sel.unwrap(); Some(self.locate(Pos { file: c.pos.file, off: a }, false, self.hint())) }
                else { self.step(&c, false) }
            }
            "right" | "wordright" => {
                if !shift && sel.is_some() { let (_, b) = sel.unwrap(); Some(self.locate(Pos { file: c.pos.file, off: b }, true, self.hint())) }
                else { self.step(&c, true) }
            }
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
            if new.pos.file != c.pos.file { return; }
            if self.anchor.is_none() { self.anchor = Some(c.pos); }
        } else { self.anchor = None; }
        self.caret = Some(new);
        if self.anchor == Some(new.pos) { self.anchor = None; }
        self.lock = None;
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
            Some(e) => { self.elem = Some(ElemSel { file, range: e.range, name: e.name }); }
            None => self.msg = Some(("이 글은 따로 떼어 낼 요소가 아닙니다. 글자를 끌어서 선택해 지워 주세요.".into(), "info")),
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
            Some(e) => { let file = el.file; self.elem = Some(ElemSel { file, range: e.range, name: e.name }); }
            None => self.msg = Some(("더 넓힐 수 없습니다.".into(), "info")),
        }
    }

    fn apply_file(&mut self, file: FileId, new_text: String) -> bool {
        let Some(before) = self.world.text_of(file) else { return false };
        let w = Work { text: new_text, caret: 0, anchor: 0 };
        if !self.commit(file, &before, &w) { return false; }
        self.push_undo(Step { file, before, after: w.text, caret_before: None, caret_after: None, typing: false, time: Instant::now() });
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
                    self.elem = Some(ElemSel { file: el.file, range: r, name });
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
        })
    }

    // ───────────── 고치기

    fn flush_comp(&mut self) {
        if let Some(cp) = self.comp.take() {
            if let Some(now) = self.world.text_of(cp.file) {
                if now != cp.base_text {
                    self.push_undo(Step { file: cp.file, before: cp.base_text, after: now, caret_before: cp.caret_before, caret_after: self.caret.map(|c| c.pos), typing: false, time: Instant::now() });
                }
            }
        }
    }

    fn push_undo(&mut self, st: Step) {
        self.redo.clear();
        if st.typing {
            if let Some(last) = self.undo.last_mut() {
                if last.typing && last.file == st.file && last.caret_after == st.caret_before && st.time.duration_since(last.time).as_millis() < 1500 {
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
        Some((c.pos.file, Work { text, caret: c.pos.off, anchor }))
    }

    /// 연산을 적용하고 다시 조판한다. 조판이 깨지면 되돌린다.
    fn apply(&mut self, typing: bool, aff_end: bool, f: impl FnOnce(&mut Work) -> edit::R<()>) -> bool {
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
            self.push_undo(Step { file, before, after: w.text, caret_before, caret_after: self.caret.map(|c| c.pos), typing, time: Instant::now() });
        }
        true
    }

    fn commit(&mut self, file: FileId, before: &str, w: &Work) -> bool {
        if w.text == before { return true; }
        self.set_text(file, &w.text);
        if let Err(e) = self.compile() {
            self.set_text(file, before);
            let _ = self.compile();
            self.msg = Some((format!("이 수정은 조판 오류가 나서 적용하지 않았습니다. ({e})"), "error"));
            return false;
        }
        true
    }

    fn compose(&mut self, text: &str, end: bool) {
        if self.comp.is_none() {
            let Some((file, w)) = self.work() else { return };
            self.comp = Some(Comp { file, base_text: w.text, base_caret: w.caret, base_anchor: w.anchor, caret_before: self.caret.map(|c| c.pos) });
        }
        let cp = self.comp.as_ref().unwrap();
        let file = cp.file;
        let mut w = Work { text: cp.base_text.clone(), caret: cp.base_caret, anchor: cp.base_anchor };
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
                req["extend"].as_bool().unwrap_or(false), req["dbl"].as_bool().unwrap_or(false)),
            "move" => self.mv(req["dir"].as_str().unwrap_or(""), req["shift"].as_bool().unwrap_or(false)),
            "insert" => {
                let typing = !text.chars().any(|c| c.is_whitespace()) && self.sel_range().is_none();
                self.apply(typing, true, |w| edit::insert(w, text));
            }
            "compose" => self.compose(text, false),
            "composeEnd" => self.compose(text, true),
            "backspace" => { self.apply(false, true, edit::backspace); }
            "delete" => { self.apply(false, false, edit::delete); }
            "enter" => { self.apply(false, false, edit::enter); }
            "linebreak" => { self.apply(false, false, edit::linebreak); }
            "tab" => { let wd = self.tab_width(); self.apply(false, true, |w| edit::tab(w, wd)); }
            "bold" => { let u = self.under_fn.clone(); self.apply(false, true, |w| edit::toggle(w, Style::Bold, &u)); }
            "underline" => { let u = self.under_fn.clone(); self.apply(false, true, |w| edit::toggle(w, Style::Under, &u)); }
            "indent" => { self.apply(false, true, |w| edit::hanging(w, 1)); }
            "outdent" => { self.apply(false, true, |w| edit::hanging(w, -1)); }
            "undo" => self.undo_redo(true),
            "redo" => self.undo_redo(false),
            "save" => { self.save(); }
            "pick" => self.pick(req["page"].as_u64().unwrap_or(0) as usize, req["x"].as_f64().unwrap_or(0.0) as f32, req["y"].as_f64().unwrap_or(0.0) as f32),
            "elemGrow" => self.elem_grow(),
            "elemDelete" => self.elem_delete(),
            "elemMove" => self.elem_move(req["dx"].as_f64().unwrap_or(0.0) as f32, req["dy"].as_f64().unwrap_or(0.0) as f32),
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
        let mut fmt = (false, false);
        let mut where_ = Value::Null;
        if let Some(c) = &self.caret {
            if let Some(src) = self.world.source_of(c.pos.file) {
                let (a, b) = self.sel_range().unwrap_or((c.pos.off, c.pos.off));
                fmt = edit::fmt_state(src.text(), a, b);
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
            "fmt": { "bold": fmt.0, "underline": fmt.1 },
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

    /// 지면에서 글을 찾는다(쪽, 첫 글자·끝 글자 자리).
    pub fn find(&self, q: &str) -> Value {
        let mut out = vec![];
        if q.is_empty() { return json!(out); }
        for (p, &(a, b)) in self.ix.page_ranges.iter().enumerate() {
            let mut text = String::new();
            let mut map: Vec<(usize, usize)> = vec![];
            for gi in a..b { map.push((text.len(), gi)); text.push_str(self.ix.text_of(gi)); }
            let mut from = 0;
            while let Some(i) = text[from..].find(q) {
                let (s, e) = (from + i, from + i + q.len());
                let g0 = map[map.partition_point(|m| m.0 <= s).saturating_sub(1)].1;
                let g1 = map[map.partition_point(|m| m.0 < e).saturating_sub(1)].1;
                let (ga, gb) = (self.glyph(g0), self.glyph(g1));
                out.push(json!({ "page": p, "x0": ga.x0, "x1": gb.x1, "y": ga.y - ga.size * 0.3, "y1": gb.y - gb.size * 0.3, "kind": ga.kind }));
                from = e;
                if out.len() >= 50 { return json!(out); }
            }
        }
        json!(out)
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
