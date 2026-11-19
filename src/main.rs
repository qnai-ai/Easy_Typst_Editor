#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! 쉬운 지면 편집기 — Typst 교재를 지면에서 바로 고치는 앱 (Windows · macOS · Linux).

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
/// LaTeX 수식 → Typst 수식 변환기(tex2typst, Apache-2.0). 라이선스는 ui/vendor/tex2typst.LICENSE.txt
const TEX2TYPST_JS: &str = include_str!("../ui/vendor/tex2typst.min.js");
const TEX2TYPST_LICENSE: &str = include_str!("../ui/vendor/tex2typst.LICENSE.txt");

/// 설정·최근 목록·웹뷰 자료를 두는 폴더.
/// Windows `%LOCALAPPDATA%`, macOS `~/Library/Application Support`, Linux `~/.local/share`(XDG_DATA_HOME) 아래.
pub fn data_dir() -> PathBuf {
    let base = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir);
    let d = base.join("EasyTypstEditor");
    let _ = std::fs::create_dir_all(&d);
    d
}

struct App {
    session: Mutex<Option<Session>>,
    fonts: OnceLock<Arc<FontStore>>,
    /// 마지막으로 열지 못한 까닭. 명령줄(파일 더블클릭)로 열다 실패했을 때 첫 화면에 보여 준다.
    failed: Mutex<Option<Value>>,
    /// 명령줄로 받은 문서를 조판하는 중이면 그 파일 이름. 그동안 화면은 '조판하는 중'을 보여 준다.
    loading: Mutex<Option<String>>,
}

impl App {
    fn fonts(&self) -> Arc<FontStore> { self.fonts.get_or_init(|| Arc::new(world::load_fonts())).clone() }

    fn open(&self, path: &std::path::Path) -> Value {
        match Session::open(path, self.fonts()) {
            Ok(mut s) => {
                add_recent(&s.title, &s.main_path);
                *self.failed.lock().unwrap() = None;
                let v = s.view();
                *self.session.lock().unwrap() = Some(s);
                v
            }
            Err(problems) => {
                let list: Vec<Value> = problems.iter().map(|p| json!({ "title": lang::tr(&p.title), "detail": p.detail, "hint": lang::tr(&p.hint) })).collect();
                *self.session.lock().unwrap() = None;
                let v = json!({ "open": false, "problems": list, "msg": "문서를 열지 못했습니다.", "msgKind": "error" });
                *self.failed.lock().unwrap() = Some(v.clone());
                v
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
                let v = match g.as_mut() {
                    Some(s) => s.view(),
                    None => match app.loading.lock().unwrap().clone() {
                        Some(name) => json!({ "open": false, "loading": true, "name": name }),
                        None => app.failed.lock().unwrap().clone().unwrap_or_else(|| json!({ "open": false })),
                    },
                };
                drop(g);
                respond_json(req, &v);
            }
            "/vendor/tex2typst.js" => {
                let _ = req.respond(Response::from_string(TEX2TYPST_JS)
                    .with_header(header("Content-Type", "text/javascript; charset=utf-8"))
                    .with_header(header("Cache-Control", "max-age=31536000, immutable")));
            }
            "/vendor/tex2typst.LICENSE.txt" => {
                let _ = req.respond(Response::from_string(TEX2TYPST_LICENSE)
                    .with_header(header("Content-Type", "text/plain; charset=utf-8")));
            }
            "/api/recent" => respond_json(req, &json!(load_recent())),
            "/api/lang" => {
                let ok = jb["lang"].as_str().is_some_and(lang::set);
                respond_json(req, &json!({ "ok": ok, "lang": lang::code() }));
            }
            "/api/open" => {
                let chosen = match jb["path"].as_str() {
                    Some(p) => Some(PathBuf::from(p)),
                    None => rfd::FileDialog::new().set_title(lang::tr("문서의 main.typ 고르기")).add_filter(lang::tr("Typst 원고"), &["typ"]).pick_file(),
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
            "/api/pdf" => {
                // 저장 위치는 typst compile 과 같게: main.typ 옆의 main.pdf 를 먼저 권한다.
                let target = app.session.lock().unwrap().as_ref().map(|s| s.main_path.clone());
                let v = match target {
                    None => json!({ "ok": false }),
                    Some(main) => {
                        let name = format!("{}.pdf", main.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "document".into()));
                        let mut dlg = rfd::FileDialog::new().set_title(lang::tr("PDF로 저장")).add_filter("PDF", &["pdf"]).set_file_name(&name);
                        if let Some(dir) = main.parent() { dlg = dlg.set_directory(dir); }
                        match dlg.save_file() {
                            None => json!({ "ok": false, "cancelled": true }),
                            Some(mut out) => {
                                if out.extension().is_none() { out.set_extension("pdf"); }
                                let bytes = app.session.lock().unwrap().as_mut().map(|s| s.export_pdf());
                                match bytes {
                                    Some(Ok(b)) => match std::fs::write(&out, b) {
                                        Ok(()) => json!({ "ok": true, "msg": "PDF로 저장했습니다: ", "file": out.display().to_string() }),
                                        Err(e) => json!({ "ok": false, "msg": "PDF를 쓰지 못했습니다. 그 PDF가 다른 프로그램에서 열려 있으면 닫고 다시 해 주세요.", "detail": e.to_string() }),
                                    },
                                    Some(Err(e)) => json!({ "ok": false, "msg": "PDF를 만들지 못했습니다.", "detail": e }),
                                    None => json!({ "ok": false }),
                                }
                            }
                        }
                    }
                };
                respond_json(req, &v);
            }
            "/api/pickImage" => {
                let p = rfd::FileDialog::new().set_title(lang::tr("바꿀 그림 파일 고르기"))
                    .add_filter(lang::tr("그림"), &["png", "jpg", "jpeg", "gif", "svg", "webp", "pdf"]).pick_file();
                respond_json(req, &json!({ "path": p.map(|p| p.display().to_string()) }));
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
    match ask_save() {
        Some(true) => app.session.lock().unwrap().as_mut().map(|s| s.save()).unwrap_or(true),
        Some(false) => true,
        None => false,
    }
}

/// "저장할까요?" 알림창. 저장 Some(true) / 저장 안 함 Some(false) / 취소 None.
fn ask_save_native() -> Option<bool> {
    let r = rfd::MessageDialog::new()
        .set_title(lang::tr("쉬운 지면 편집기"))
        .set_description(lang::tr("저장하지 않은 변경이 있습니다. 저장할까요?"))
        .set_buttons(rfd::MessageButtons::YesNoCancel)
        .show();
    match r {
        rfd::MessageDialogResult::Yes => Some(true),
        rfd::MessageDialogResult::No => Some(false),
        _ => None,
    }
}

#[cfg(not(target_os = "linux"))]
fn ask_save() -> Option<bool> { ask_save_native() }

/// Linux: rfd의 알림창은 zenity가 깔려 있어야 뜬다. 그래서 창이 이미 쓰는 GTK로 직접 띄운다.
/// GTK는 창을 만든 스레드에서만 쓸 수 있으므로, 다른 스레드(화면 서버)에서 부르면 그 스레드로 넘긴다.
#[cfg(target_os = "linux")]
fn ask_save() -> Option<bool> {
    use gtk::prelude::*;
    fn show() -> Option<bool> {
        let parent = gtk::Window::list_toplevels().into_iter()
            .filter_map(|w| w.downcast::<gtk::Window>().ok()).find(|w| w.is_visible());
        let d = gtk::MessageDialog::new(parent.as_ref(), gtk::DialogFlags::MODAL, gtk::MessageType::Question,
            gtk::ButtonsType::None, &lang::tr("저장하지 않은 변경이 있습니다. 저장할까요?"));
        d.set_title(&lang::tr("쉬운 지면 편집기"));
        d.add_button(&lang::tr("저장 안 함"), gtk::ResponseType::No);
        d.add_button(&lang::tr("취소"), gtk::ResponseType::Cancel);
        d.add_button(&lang::tr("저장"), gtk::ResponseType::Yes);
        d.set_default_response(gtk::ResponseType::Yes);
        let r = d.run();
        d.close();
        match r {
            gtk::ResponseType::Yes => Some(true),
            gtk::ResponseType::No => Some(false),
            _ => None,
        }
    }
    if !gtk::is_initialized() { return ask_save_native(); }   // --serve: 창이 없다
    if gtk::is_initialized_main_thread() { return show(); }
    let (tx, rx) = std::sync::mpsc::channel();
    gtk::glib::MainContext::default().invoke(move || { let _ = tx.send(show()); });
    rx.recv().ok().flatten()
}

/// 파일 관리자에서 장별 원고(content/u01.typ 등)를 열었을 때: 같은 폴더나 위 폴더의 main.typ를 대신 연다.
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
    // --page N: 연 문서의 N쪽(PDF의 쪽 순서, 1부터)을 보여 준다
    let mut page: Option<usize> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--serve" => { headless = true; port = args.get(i + 1).and_then(|p| p.parse().ok()).unwrap_or(47615); i += 1; }
            "--page" => { page = args.get(i + 1).and_then(|p| p.parse().ok()).filter(|&n| n > 0); i += 1; }
            a => file = Some(find_main(PathBuf::from(a))),
        }
        i += 1;
    }
    let app = Arc::new(App { session: Mutex::new(None), fonts: OnceLock::new(), failed: Mutex::new(None), loading: Mutex::new(None) });
    let server = Server::http(("127.0.0.1", port)).expect("포트를 열지 못했습니다");
    let port = server.server_addr().to_ip().map(|a| a.port()).unwrap_or(port);
    {
        let app = app.clone();
        std::thread::spawn(move || { app.fonts(); });
    }
    let url = format!("http://127.0.0.1:{port}/");
    // 처음 열 때만 그 쪽으로 간다(다시 불러올 때는 평소처럼 고치던 자리로)
    let first_url = match page { Some(n) => format!("{url}?page={n}"), None => url.clone() };
    if headless {
        if let Some(f) = &file { app.open(f); }
        println!("{first_url}");
        serve(server, app);
        return;
    }
    {
        // 큰 문서는 첫 조판에 몇 초가 걸린다. 화면을 먼저 띄우고(서버를 바로 열고) 조판은 따로 돌린다.
        if let Some(f) = file.clone() {
            *app.loading.lock().unwrap() = Some(f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
            let app = app.clone();
            std::thread::spawn(move || {
                app.open(&f);
                *app.loading.lock().unwrap() = None;
            });
        }
        let app = app.clone();
        std::thread::spawn(move || serve(server, app));
    }
    run_window(&url, &first_url, app);
}

/// 창 밖에서 창의 스레드로 보내는 알림.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
enum Wake {
    /// 끝내기(macOS의 ⌘Q)
    Quit,
    /// 다른 교재를 열었으니 화면을 다시 불러온다
    Reload,
}

/// macOS의 메뉴 막대. 편집 메뉴가 없으면 ⌘V·⌘C 같은 글쇠가 웹뷰까지 가지 않는다.
/// 편집기가 직접 받는 글쇠(⌘C, ⌘Z, ⌘S …)는 웹뷰가 먼저 받으므로 그대로이고,
/// 나머지(붙여넣기, 찾기 칸 안의 복사·실행 취소 등)만 메뉴를 거쳐 처리된다.
#[cfg(target_os = "macos")]
fn mac_menu(proxy: tao::event_loop::EventLoopProxy<Wake>) {
    use muda::accelerator::{Accelerator, Code, CMD_OR_CTRL};
    use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem as Item, Submenu};

    let t = |ko: &str| lang::tr(ko);
    let name = t("쉬운 지면 편집기");
    // 끝내기는 미리 만들어진 항목을 쓰지 않는다: 저장 안 한 변경이 있으면 먼저 물어야 한다
    let quit = MenuItem::with_id("quit", lang::f("{0} 끝내기", &[&name]), true, Some(Accelerator::new(CMD_OR_CTRL, Code::KeyQ)));
    let app_menu = Submenu::new(&name, true);
    let edit_menu = Submenu::new(t("편집"), true);
    let window_menu = Submenu::new(t("창"), true);
    let menu = Menu::new();
    let built = app_menu.append_items(&[
        &Item::about(None, None), &Item::separator(),
        &Item::hide(None), &Item::hide_others(None), &Item::show_all(None), &Item::separator(),
        &quit,
    ]).and_then(|_| edit_menu.append_items(&[
        &Item::undo(Some(t("실행 취소").as_str())), &Item::redo(Some(t("다시 실행").as_str())), &Item::separator(),
        &Item::cut(Some(t("잘라내기").as_str())), &Item::copy(Some(t("복사").as_str())), &Item::paste(Some(t("붙여넣기").as_str())),
        &Item::select_all(Some(t("모두 선택").as_str())),
    ])).and_then(|_| window_menu.append_items(&[&Item::minimize(None), &Item::close_window(None)]))
        .and_then(|_| menu.append_items(&[&app_menu, &edit_menu, &window_menu]));
    if built.is_err() { return; }
    menu.init_for_nsapp();
    window_menu.set_as_windows_menu_for_nsapp();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        if e.id == "quit" { let _ = proxy.send_event(Wake::Quit); }
    }));
    std::mem::forget(menu);   // 앱이 끝날 때까지 둔다
}

fn run_window(url: &str, first_url: &str, app: Arc<App>) {
    use tao::dpi::LogicalSize;
    use tao::event::{Event, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tao::window::WindowBuilder;
    use wry::WebViewBuilder;
    #[cfg(windows)]
    use wry::WebViewBuilderExtWindows;

    let event_loop = EventLoopBuilder::<Wake>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    #[cfg(target_os = "macos")]
    mac_menu(proxy.clone());
    let window = WindowBuilder::new()
        .with_title(lang::tr("쉬운 지면 편집기"))
        .with_inner_size(LogicalSize::new(1280.0, 900.0))
        .build(&event_loop)
        .expect("창을 만들지 못했습니다");
    let data = data_dir().join("webview");
    let mut ctx = wry::WebContext::new(Some(data));
    let builder = WebViewBuilder::new_with_web_context(&mut ctx).with_url(first_url);
    // 브라우저의 가속키(Ctrl+F, F3, Ctrl+P, F5, Ctrl+R …)는 끈다: 같은 글쇠를 편집기의 단축키로 쓴다
    #[cfg(windows)]
    let builder = builder.with_browser_accelerator_keys(false);
    // Linux(WebKitGTK): 지면에서 복사·붙여넣기를 할 수 있게 클립보드를 열어 준다
    #[cfg(target_os = "linux")]
    let builder = builder.with_clipboard(true);
    #[cfg(not(target_os = "linux"))]
    let webview = builder.build(&window);
    // Linux: 창의 GTK 상자 안에 넣는다(창 핸들로 만드는 방식은 X11에서만 되고 Wayland에서는 안 된다)
    #[cfg(target_os = "linux")]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        builder.build_gtk(window.default_vbox().expect("창에 GTK 상자가 없습니다"))
    };
    let webview = webview.expect(if cfg!(windows) {
        "Could not create the web view (Microsoft Edge WebView2 Runtime is required)"
    } else if cfg!(target_os = "macos") {
        "Could not create the web view (WebKit)"
    } else {
        "Could not create the web view (WebKitGTK 4.1 is required: libwebkit2gtk-4.1)"
    });
    let url = url.to_string();
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        let may_leave = || {
            let dirty = app.session.lock().unwrap().as_ref().is_some_and(|s| s.is_dirty());
            !dirty || confirm_discard(&app)
        };
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } | Event::UserEvent(Wake::Quit) => {
                if may_leave() { *control_flow = ControlFlow::Exit; }
            }
            // macOS: Finder에서 .typ 파일을 이 앱으로 열었을 때(파일 이름이 명령줄이 아니라 이 알림으로 온다)
            Event::Opened { urls } => {
                if let Some(p) = urls.iter().find_map(|u| u.to_file_path().ok()) {
                    if may_leave() {
                        let (app, proxy) = (app.clone(), proxy.clone());
                        std::thread::spawn(move || { app.open(&find_main(p)); let _ = proxy.send_event(Wake::Reload); });
                    }
                }
            }
            Event::UserEvent(Wake::Reload) => { let _ = webview.load_url(&url); }
            _ => {}
        }
    });
}
