//! Typst 조판 세계: 프로젝트 폴더의 파일을 메모리에 올려 두고(편집은 메모리에서만),
//! 저장할 때에만 디스크에 쓴다.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{Datelike, Local};
use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};
use typst_kit::fonts::FontStore;

pub struct Entry {
    /// 실제 파일 경로(프로젝트·패키지 안).
    pub path: PathBuf,
    /// 디스크에서 읽은 그대로의 바이트.
    pub bytes: Bytes,
    /// 소스로 읽힌 적이 있으면 그 소스(편집 반영본).
    pub source: Option<Source>,
    /// 마지막으로 디스크와 일치했던 본문(저장 여부·외부 변경 판단용).
    pub disk_text: Option<String>,
    /// 파일 머리에 BOM이 있었는지.
    pub bom: bool,
    /// 프로젝트 안 파일인지(패키지가 아닌지).
    pub in_project: bool,
}

pub struct ProjectWorld {
    pub root: PathBuf,
    pub main: FileId,
    library: LazyHash<Library>,
    fonts: std::sync::Arc<FontStore>,
    pub files: Mutex<HashMap<FileId, FileResult<Entry>>>,
    today: (i32, u8, u8),
    /// 교재 폴더 주변에서 찾은 패키지 위치
    nearby: Mutex<HashMap<String, Option<PathBuf>>>,
}

pub fn load_fonts() -> FontStore {
    let mut fonts = FontStore::new();
    fonts.extend(typst_kit::fonts::system());
    fonts.extend(typst_kit::fonts::embedded());
    fonts
}

impl ProjectWorld {
    pub fn new(root: &Path, main_path: &Path, fonts: std::sync::Arc<FontStore>) -> Result<Self, String> {
        let vpath = VirtualPath::virtualize(root, main_path)
            .map_err(|e| format!("주 파일 경로를 해석하지 못했습니다: {e}"))?;
        let main = RootedPath::new(VirtualRoot::Project, vpath).intern();
        let now = Local::now();
        Ok(Self {
            root: root.to_path_buf(),
            main,
            library: LazyHash::new(Library::default()),
            fonts,
            files: Mutex::new(HashMap::new()),
            today: (now.year(), now.month() as u8, now.day() as u8),
            nearby: Mutex::new(HashMap::new()),
        })
    }

    fn resolve(&self, id: FileId) -> FileResult<(PathBuf, bool)> {
        match id.root() {
            VirtualRoot::Project => {
                let p = id.vpath().realize(&self.root).map_err(FileError::from)?;
                Ok((p, true))
            }
            VirtualRoot::Package(spec) => {
                let sub = format!("typst/packages/{}/{}/{}", spec.namespace, spec.name, spec.version);
                let mut bases = vec![];
                if let Some(d) = std::env::var_os("APPDATA") { bases.push(PathBuf::from(d)); }
                if let Some(d) = std::env::var_os("LOCALAPPDATA") { bases.push(PathBuf::from(d)); }
                for b in bases {
                    let dir = b.join(&sub);
                    if dir.is_dir() {
                        let p = id.vpath().realize(&dir).map_err(FileError::from)?;
                        return Ok((p, false));
                    }
                }
                // typst에 이어 두지 않은 패키지: 교재 폴더 주변(위 폴더들의 아래)에서 typst.toml로 찾는다
                if let Some(dir) = self.find_nearby_package(&spec.name, &spec.version.to_string()) {
                    let p = id.vpath().realize(&dir).map_err(FileError::from)?;
                    return Ok((p, false));
                }
                Err(FileError::Other(Some(
                    crate::lang::f("패키지 {0} 가 이 컴퓨터에 없습니다", &[&spec]).into(),
                )))
            }
        }
    }

    fn find_nearby_package(&self, name: &str, version: &str) -> Option<PathBuf> {
        let key = format!("{name}:{version}");
        if let Some(hit) = self.nearby.lock().unwrap().get(&key) { return hit.clone(); }
        fn is_match(dir: &Path, name: &str, version: &str) -> bool {
            let Ok(t) = std::fs::read_to_string(dir.join("typst.toml")) else { return false };
            let val = |k: &str| t.lines().find_map(|l| {
                let (a, b) = l.split_once('=')?;
                if a.trim() == k { Some(b.trim().trim_matches('"').to_string()) } else { None }
            });
            val("name").as_deref() == Some(name) && val("version").as_deref() == Some(version)
        }
        fn walk(dir: &Path, depth: usize, name: &str, version: &str) -> Option<PathBuf> {
            if is_match(dir, name, version) { return Some(dir.to_path_buf()); }
            if depth == 0 { return None; }
            for e in std::fs::read_dir(dir).ok()?.flatten() {
                let p = e.path();
                let n = e.file_name().to_string_lossy().to_string();
                if !p.is_dir() || n.starts_with('.') || n.starts_with('_') || n == "node_modules" { continue; }
                if let Some(f) = walk(&p, depth - 1, name, version) { return Some(f); }
            }
            None
        }
        let mut found = None;
        let mut dir = self.root.parent();
        for _ in 0..2 {
            let Some(d) = dir else { break };
            if let Some(f) = walk(d, 3, name, version) { found = Some(f); break; }
            dir = d.parent();
        }
        self.nearby.lock().unwrap().insert(key, found.clone());
        found
    }

    fn load(&self, id: FileId) -> FileResult<Entry> {
        let (path, in_project) = self.resolve(id)?;
        let f = |e| FileError::from_io(e, &path);
        if std::fs::metadata(&path).map_err(f)?.is_dir() {
            return Err(FileError::IsDirectory);
        }
        let data = std::fs::read(&path).map_err(f)?;
        Ok(Entry { path, bytes: Bytes::new(data), source: None, disk_text: None, bom: false, in_project })
    }

    fn with_entry<T>(&self, id: FileId, f: impl FnOnce(&mut Entry) -> FileResult<T>) -> FileResult<T> {
        let mut files = self.files.lock().unwrap();
        let slot = files.entry(id).or_insert_with(|| self.load(id));
        match slot {
            Ok(e) => f(e),
            Err(e) => Err(e.clone()),
        }
    }

    /// 읽기에 실패했던 파일은 다음 조판 때 다시 읽도록 지운다.
    pub fn forget_failed(&self) {
        self.files.lock().unwrap().retain(|_, v| v.is_ok());
    }

    pub fn source_of(&self, id: FileId) -> Option<Source> {
        self.source(id).ok()
    }

    /// 메모리의 소스를 고친다. 디스크에는 쓰지 않는다.
    pub fn edit(&self, id: FileId, range: std::ops::Range<usize>, with: &str) -> bool {
        let mut files = self.files.lock().unwrap();
        if let Some(Ok(e)) = files.get_mut(&id) {
            if let Some(src) = e.source.as_mut() {
                if range.end <= src.text().len()
                    && src.text().is_char_boundary(range.start)
                    && src.text().is_char_boundary(range.end)
                {
                    src.edit(range, with);
                    return true;
                }
            }
        }
        false
    }

    /// 파일 본문을 통째로 바꾼다(실행 취소용).
    pub fn set_text(&self, id: FileId, text: &str) {
        let mut files = self.files.lock().unwrap();
        if let Some(Ok(e)) = files.get_mut(&id) {
            if let Some(src) = e.source.as_mut() {
                src.replace(text);
            }
        }
    }

    pub fn text_of(&self, id: FileId) -> Option<String> {
        self.source_of(id).map(|s| s.text().to_string())
    }

    /// 밑줄에 쓸 함수 이름: 교재 템플릿이 `u`를 정의해 두었으면 `u`, 아니면 `underline`.
    pub fn underline_fn(&self) -> String {
        let files = self.files.lock().unwrap();
        for e in files.values().flatten() {
            if let Some(src) = &e.source {
                if e.in_project && src.text().contains("#let u(") { return "u".into(); }
            }
        }
        "underline".into()
    }

    /// 디스크와 달라진 소스 목록.
    pub fn dirty_files(&self) -> Vec<FileId> {
        let files = self.files.lock().unwrap();
        let mut out = vec![];
        for (id, e) in files.iter() {
            if let Ok(e) = e {
                if let (Some(src), Some(disk)) = (&e.source, &e.disk_text) {
                    if src.text() != disk { out.push(*id); }
                }
            }
        }
        out
    }

    /// 바뀐 파일을 디스크에 쓴다. 쓰기 전에 원래 파일을 `backup` 폴더에 복사해 둔다.
    /// 그 사이 다른 프로그램이 파일을 바꿨으면 아무것도 쓰지 않는다.
    pub fn save_all(&self, backup: &Path) -> Result<Vec<String>, String> {
        const BOM: &str = "\u{feff}";
        let dirty = self.dirty_files();
        let mut files = self.files.lock().unwrap();
        for id in &dirty {
            if let Some(Ok(e)) = files.get(id) {
                let now = std::fs::read(&e.path).map_err(|err| crate::lang::f("{0} 파일을 읽지 못했습니다: {1}", &[&e.path.display(), &err]))?;
                let now = String::from_utf8_lossy(&now).to_string();
                let now = now.strip_prefix(BOM).unwrap_or(&now);
                if Some(now) != e.disk_text.as_deref() {
                    let name = e.path.file_name().unwrap_or_default().to_string_lossy();
                    return Err(crate::lang::f("저장하지 않았습니다. 교재를 연 뒤에 다른 프로그램이 {0} 파일을 바꿨습니다. 교재를 다시 열어 주세요.", &[&name]));
                }
            }
        }
        let mut names = vec![];
        for id in &dirty {
            if let Some(Ok(e)) = files.get_mut(id) {
                let Some(src) = &e.source else { continue };
                let bpath = backup.join(id.vpath().get_without_slash());
                if let Some(parent) = bpath.parent() { let _ = std::fs::create_dir_all(parent); }
                let _ = std::fs::copy(&e.path, &bpath);
                let mut data = Vec::new();
                if e.bom { data.extend_from_slice(BOM.as_bytes()); }
                data.extend_from_slice(src.text().as_bytes());
                std::fs::write(&e.path, &data).map_err(|err| crate::lang::f("{0} 파일에 쓰지 못했습니다: {1}", &[&e.path.display(), &err]))?;
                e.disk_text = Some(src.text().to_string());
                e.bytes = Bytes::new(data);
                names.push(e.path.file_name().unwrap_or_default().to_string_lossy().to_string());
            }
        }
        Ok(names)
    }

    pub fn path_of(&self, id: FileId) -> Option<PathBuf> {
        let files = self.files.lock().unwrap();
        match files.get(&id) { Some(Ok(e)) => Some(e.path.clone()), _ => None }
    }

    pub fn in_project(&self, id: FileId) -> bool {
        let files = self.files.lock().unwrap();
        matches!(files.get(&id), Some(Ok(e)) if e.in_project)
    }
}

impl World for ProjectWorld {
    fn library(&self) -> &LazyHash<Library> { &self.library }
    fn book(&self) -> &LazyHash<FontBook> { self.fonts.book() }
    fn main(&self) -> FileId { self.main }

    fn source(&self, id: FileId) -> FileResult<Source> {
        self.with_entry(id, |e| {
            if e.source.is_none() {
                let s = std::str::from_utf8(e.bytes.as_slice()).map_err(|_| FileError::InvalidUtf8)?;
                let (bom, text) = match s.strip_prefix('\u{feff}') { Some(t) => (true, t), None => (false, s) };
                e.bom = bom;
                e.disk_text = Some(text.to_string());
                e.source = Some(Source::new(id, text.to_string()));
            }
            Ok(e.source.clone().unwrap())
        })
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        self.with_entry(id, |e| match &e.source {
            Some(src) => Ok(Bytes::from_string(src.text().to_string())),
            None => Ok(e.bytes.clone()),
        })
    }

    fn font(&self, index: usize) -> Option<Font> { self.fonts.font(index) }

    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        Datetime::from_ymd(self.today.0, self.today.1, self.today.2)
    }
}
