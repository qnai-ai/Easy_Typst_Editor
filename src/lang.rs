//! 화면 언어 (UI language).
//!
//! 지원 언어: 한국어(ko), English(en), 中文(zh), 日本語(ja), Español(es).
//! 처음엔 시스템(Windows · macOS · Linux)의 표시 언어를 따르고, 화면 오른쪽 위 언어 메뉴(🌐)에서 고르면 기억한다.
//! 환경 변수 `EASY_TYPST_LANG=ko|en|zh|ja|es` 가 있으면 그것이 우선한다.
//! 엔진의 안내문은 한국어 원문으로 만들고, 화면으로 보내기 직전에 `ui/i18n.json` 으로 옮긴다.
//! 화면(ui/index.html)도 같은 i18n.json 을 받아 쓴다.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::{OnceLock, RwLock};

use serde_json::Value;

pub const LANGS: &[&str] = &["ko", "en", "zh", "ja", "es"];
pub const I18N_JSON: &str = include_str!("../ui/i18n.json");

static CURRENT: RwLock<Option<&'static str>> = RwLock::new(None);

fn settings_path() -> std::path::PathBuf { crate::data_dir().join("settings.json") }

fn saved_lang() -> Option<&'static str> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(settings_path()).ok()?).ok()?;
    let l = v.get("lang")?.as_str()?;
    LANGS.iter().find(|c| **c == l).copied()
}

/// 지금 화면 언어. 우선순위: 환경 변수 EASY_TYPST_LANG > 화면에서 고른 언어(settings.json) > 시스템 표시 언어.
pub fn code() -> &'static str {
    if let Some(c) = *CURRENT.read().unwrap() { return c; }
    let c = std::env::var("EASY_TYPST_LANG").ok()
        .and_then(|v| { let v = v.to_ascii_lowercase(); LANGS.iter().find(|c| v.starts_with(**c)).copied() })
        .or_else(saved_lang)
        .unwrap_or_else(system_lang);
    *CURRENT.write().unwrap() = Some(c);
    c
}

/// 화면에서 언어를 고르면: 바로 바꾸고 다음 실행을 위해 기억한다.
pub fn set(lang: &str) -> bool {
    let Some(c) = LANGS.iter().find(|c| **c == lang).copied() else { return false };
    *CURRENT.write().unwrap() = Some(c);
    let mut v: Value = std::fs::read_to_string(settings_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| serde_json::json!({}));
    v["lang"] = Value::String(c.to_string());
    let _ = std::fs::write(settings_path(), serde_json::to_string_pretty(&v).unwrap_or_default());
    true
}

#[cfg(windows)]
fn system_lang() -> &'static str {
    #[link(name = "kernel32")]
    extern "system" { fn GetUserDefaultUILanguage() -> u16; }
    match unsafe { GetUserDefaultUILanguage() } & 0x3ff {
        0x12 => "ko",
        0x04 => "zh",
        0x11 => "ja",
        0x0a => "es",
        _ => "en",
    }
}

/// macOS: 시스템 설정의 선호 언어. Linux: LANGUAGE · LC_ALL · LC_MESSAGES · LANG.
#[cfg(not(windows))]
fn system_lang() -> &'static str {
    let l = sys_locale::get_locale().unwrap_or_default().to_ascii_lowercase();
    LANGS.iter().find(|c| l.starts_with(**c)).copied().unwrap_or("en")
}

struct Table { exact: HashMap<String, String>, parts: Vec<(String, String)> }

fn table() -> &'static Table {
    static T: OnceLock<HashMap<&'static str, Table>> = OnceLock::new();
    &T.get_or_init(|| LANGS.iter().map(|l| (*l, build(l))).collect())[code()]
}

fn build(lang: &str) -> Table {
    {
        let mut exact = HashMap::new();
        let mut parts = vec![];
        if let Ok(Value::Object(m)) = serde_json::from_str::<Value>(I18N_JSON) {
            for (ko, v) in m {
                let Some(t) = v.get(lang).and_then(|t| t.as_str()) else { continue };
                // 긴 문장 조각은 다른 글(예: 오류 설명이 붙은 안내문) 속에서도 바꾼다
                if ko.chars().count() >= 6 && !ko.contains('{') { parts.push((ko.clone(), t.to_string())); }
                exact.insert(ko, t.to_string());
            }
        }
        parts.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));
        Table { exact, parts }
    }
}

fn is_ko() -> bool { code() == "ko" }

/// 한국어 원문을 화면 언어로.
pub fn tr(ko: &str) -> String {
    if is_ko() { return ko.to_string(); }
    let t = table();
    if let Some(e) = t.exact.get(ko) { return e.clone(); }
    if !ko.chars().any(|c| ('가'..='힣').contains(&c)) { return ko.to_string(); }
    let mut out = ko.to_string();
    for (k, e) in &t.parts { if out.contains(k.as_str()) { out = out.replace(k.as_str(), e); } }
    out
}

/// `{0}`, `{1}` 자리표가 든 한국어 원문을 화면 언어로 옮기고 값을 채운다.
pub fn f(ko_template: &str, args: &[&dyn Display]) -> String {
    let mut s = if is_ko() { ko_template.to_string() } else { table().exact.get(ko_template).cloned().unwrap_or_else(|| ko_template.to_string()) };
    for (i, a) in args.iter().enumerate() { s = s.replace(&format!("{{{i}}}"), &a.to_string()); }
    s
}

/// 화면으로 보내는 JSON의 안내문을 옮긴다. 원고 글·경로는 건드리지 않는다.
pub fn translate(v: &mut Value) {
    if is_ko() { return; }
    match v {
        Value::String(s) => { let n = tr(s); if n != *s { *s = n; } }
        Value::Array(a) => a.iter_mut().for_each(translate),
        Value::Object(o) => for (k, x) in o.iter_mut() {
            if matches!(k.as_str(), "text" | "selText" | "path" | "file" | "detail" | "title" | "name") { continue; }
            translate(x);
        },
        _ => {}
    }
}
