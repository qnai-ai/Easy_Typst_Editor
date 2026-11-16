#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! 쉬운 지면 편집기 — Typst 교재를 지면에서 바로 고치는 Windows 앱.

mod edit;
mod index;
mod lang;
mod recover;
mod session;
mod world;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{json, Value};
use tiny_http::{Header, Method, Response, Server};
use typst_kit::fonts::FontStore;

use session::Session;

const INDEX_HTML: &str = include_str!("../ui/index.html");

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let d = base.join("EasyTypstEditor");
    let _ = std::fs::create_dir_all(&d);
    d
}

struct App {
    session: Mutex<Option<Session>>,
    fonts: OnceLock<Arc<FontStore>>,
}

impl App {
    fn fonts(&self) -> Arc<FontStore> { self.fonts.get_or_init(|| Arc::new(world::load_fonts())).clone() }

    fn open(&self, path: &std::path::Path) -> Value {
        match Session::open(path, self.fonts()) {
            Ok(mut s) => {
                add_recent(&s.title, &s.main_path);
                let v = s.view();
                *self.session.lock().unwrap() = Some(s);
                v
            }
            Err(problems) => {
                let list: Vec<Value> = problems.iter().map(|p| json!({ "title": lang::tr(&p.title), "detail": p.detail, "hint": lang::tr(&p.hint) })).collect();
                *self.session.lock().unwrap() = None;
                json!({ "open": false, "problems": list, "msg": "교재를 열지 못했습니다.", "msgKind": "error" })
            }
        }
    }
}

fn recent_path() -> PathBuf { data_dir().join("recent.json") }
fn load_recent() -> Vec<Value> {
    std::fs::read_to_string(recent_path()).ok().and_then(|s| serde_json::from_str::<Vec<Value>>(&s).ok()).unwrap_or_default()
        .into_iter().filter(|v| v["path"].as_str().is_some_and(|p| std::path::Path::new(p).is_file())).collect()
}
fn add_recent(title: &str, path: &std::path::Path) {
    let p = path.display().to_string();
    let mut list = load_recent();
    list.retain(|v| v["path"].as_str() != Some(&p));
    list.insert(0, json!({ "title": title, "path": p }));
    list.truncate(8);
    let _ = std::fs::write(recent_path(), serde_json::to_string_pretty(&list).unwrap_or_default());
}

fn header(k: &str, v: &str) -> Header { Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap() }

fn respond_json(req: tiny_http::Request, v: &Value) {
    let mut v = v.clone();
    lang::translate(&mut v);
    let _ = req.respond(Response::from_string(v.to_string())
        .with_header(header("Content-Type", "application/json; charset=utf-8"))
        .with_header(header("Cache-Control", "no-store")));
}

fn serve(server: Server, app: Arc<App>) {
    for mut req in server.incoming_requests() {
        let url = req.url().to_string();
        let path = url.split('?').next().unwrap_or("").to_string();
        let mut body = String::new();
        if *req.method() == Method::Post { let _ = req.as_reader().read_to_string(&mut body); }
        let jb: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        match path.as_str() {
            "/" | "/index.html" => {
                let html = INDEX_HTML.replacen("<html lang=\"ko\">", &format!("<html lang=\"{}\">", lang::code()), 1).replacen("/*I18N*/{}", lang::I18N_JSON, 1);
                let _ = req.respond(Response::from_string(html)
                    .with_header(header("Content-Type", "text/html; charset=utf-8"))
                    .with_header(header("Cache-Control", "no-store")));
            }
            "/api/state" => {
                let mut g = app.session.lock().unwrap();
                let v = match g.as_mut() { Some(s) => s.view(), None => json!({ "open": false }) };
                drop(g);
                respond_json(req, &v);
            }
            "/api/recent" => respond_json(req, &json!(load_recent())),
            "/api/lang" => {
                let ok = jb["lang"].as_str().is_some_and(lang::set);
                respond_json(req, &json!({ "ok": ok, "lang": lang::code() }));
            }
            "/api/open" => {
                let chosen = match jb["path"].as_str() {
                    Some(p) => Some(PathBuf::from(p)),
                    None => rfd::FileDialog::new().set_title(lang::tr("교재의 main.typ 고르기")).add_filter(lang::tr("Typst 원고"), &["typ"]).pick_file(),
                };
                let v = match chosen {
                    Some(p) => {
                        let dirty = app.session.lock().unwrap().as_ref().is_some_and(|s| s.is_dirty());
                        if dirty && !confirm_discard(&app) { json!({ "cancelled": true }) } else { app.open(&p) }
                    }
                    None => json!({ "cancelled": true }),
                };
                respond_json(req, &v);
            }
            "/api/act" => {
                let mut g = app.session.lock().unwrap();
                let v = match g.as_mut() { Some(s) => { s.act(&jb); s.view() } None => json!({ "open": false }) };
                drop(g);
                respond_json(req, &v);
            }
            "/api/find" => {
                let g = app.session.lock().unwrap();
                let v = match g.as_ref() { Some(s) => s.find(jb["q"].as_str().unwrap_or(""), jb["case"].as_bool().unwrap_or(false)), None => json!({ "hits": [] }) };
                drop(g);
                respond_json(req, &v);
            }
            "/api/source" => {
                let g = app.session.lock().unwrap();
                let v = match g.as_ref() { Some(s) => s.source_view(), None => json!({}) };
                drop(g);
                respond_json(req, &v);
            }
            p if p.starts_with("/page/") => {
                let i: usize = p.trim_start_matches("/page/").trim_end_matches(".svg").parse().unwrap_or(usize::MAX);
                let svg = app.session.lock().unwrap().as_ref().and_then(|s| s.page_svg(i));
                match svg {
                    Some(svg) => {
                        let _ = req.respond(Response::from_string(svg)
                            .with_header(header("Content-Type", "image/svg+xml"))
                            .with_header(header("Cache-Control", "max-age=31536000, immutable")));
                    }
                    None => { let _ = req.respond(Response::from_string("").with_status_code(404)); }
                }
            }
            _ => { let _ = req.respond(Response::from_string("").with_status_code(404)); }
        }
    }
}

/// 저장 안 한 변경이 있을 때: 저장(예) / 버림(아니오) / 취소. 계속해도 되면 true.
fn confirm_discard(app: &App) -> bool {
    let r = rfd::MessageDialog::new()
        .set_title(lang::tr("쉬운 지면 편집기"))
        .set_description(lang::tr("저장하지 않은 변경이 있습니다. 저장할까요?"))
        .set_buttons(rfd::MessageButtons::YesNoCancel)
        .show();
    match r {
        rfd::MessageDialogResult::Yes => app.session.lock().unwrap().as_mut().map(|s| s.save()).unwrap_or(true),
        rfd::MessageDialogResult::No => true,
        _ => false,
    }
}

/// 탐색기에서 장별 원고(content/u01.typ 등)를 열었을 때: 같은 폴더나 위 폴더의 main.typ를 대신 연다.
fn find_main(path: PathBuf) -> PathBuf {
    if path.file_name().is_some_and(|n| n.eq_ignore_ascii_case("main.typ")) { return path; }
    let mut dir = path.parent();
    for _ in 0..3 {
        let Some(d) = dir else { break };
        let m = d.join("main.typ");
        if m.is_file() { return m; }
        dir = d.parent();
    }
    path
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut port = 0u16;
    let mut headless = false;
    let mut file: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--serve" => { headless = true; port = args.get(i + 1).and_then(|p| p.parse().ok()).unwrap_or(47615); i += 1; }
            a => file = Some(find_main(PathBuf::from(a))),
        }
        i += 1;
    }
    let app = Arc::new(App { session: Mutex::new(None), fonts: OnceLock::new() });
    let server = Server::http(("127.0.0.1", port)).expect("포트를 열지 못했습니다");
    let port = server.server_addr().to_ip().map(|a| a.port()).unwrap_or(port);
    {
        let app = app.clone();
        std::thread::spawn(move || { app.fonts(); });
    }
    let url = format!("http://127.0.0.1:{port}/");
    if headless {
        if let Some(f) = &file { app.open(f); }
        println!("{url}");
        serve(server, app);
        return;
    }
    {
        let app = app.clone();
        let file = file.clone();
        std::thread::spawn(move || {
            if let Some(f) = &file { app.open(f); }
            serve(server, app)
        });
    }
    run_window(&url, app);
}

fn run_window(url: &str, app: Arc<App>) {
    use tao::dpi::LogicalSize;
    use tao::event::{Event, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoop};
    use tao::window::WindowBuilder;
    use wry::WebViewBuilder;
    #[cfg(windows)]
    use wry::WebViewBuilderExtWindows;

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title(lang::tr("쉬운 지면 편집기"))
        .with_inner_size(LogicalSize::new(1280.0, 900.0))
        .build(&event_loop)
        .expect("창을 만들지 못했습니다");
    let data = data_dir().join("webview");
    let mut ctx = wry::WebContext::new(Some(data));
    let builder = WebViewBuilder::new_with_web_context(&mut ctx).with_url(url);
    // 브라우저의 가속키(Ctrl+F, F3, Ctrl+P, F5, Ctrl+R …)는 끈다: 같은 글쇠를 편집기의 단축키로 쓴다
    #[cfg(windows)]
    let builder = builder.with_browser_accelerator_keys(false);
    let _webview = builder
        .build(&window)
        .expect("Could not create the web view (Microsoft Edge WebView2 Runtime is required)");
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        if let Event::WindowEvent { event: WindowEvent::CloseRequested, .. } = event {
            let dirty = app.session.lock().unwrap().as_ref().is_some_and(|s| s.is_dirty());
            if !dirty || confirm_discard(&app) { *control_flow = ControlFlow::Exit; }
        }
    });
}
