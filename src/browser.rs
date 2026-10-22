use std::{cell::{Cell, RefCell}, error::Error, os::windows::process::CommandExt, path::PathBuf, rc::Rc, time::{Duration, Instant}};
use serde::Deserialize;
use serde_json::json;
use tao::{event::{Event, WindowEvent}, event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy}, window::{Window, WindowBuilder}};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::platform::windows::WindowBuilderExtWindows;
use wry::{dpi::{LogicalPosition, LogicalSize}, MemoryUsageLevel, PageLoadEvent, Rect, WebContext, WebView, WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows};
use crate::{native::{self, Downloads}, policy};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const CREATE_NO_WINDOW: u32 = 0x08000000;
const TOOLBAR: f64 = 100.0;
const FOOTER: f64 = 28.0;
const DOWNLOAD_PANEL: f64 = 220.0;
const SLEEP_AFTER: Duration = Duration::from_secs(45);
// An explicit empty argument string also overrides wry 0.48's default flags that
// disable SmartScreen. WebView2's GPU/decode selection and sandbox stay enabled.
const BROWSER_ARGS: &str = "";
const FORCE_GPU_ARGS: &str = "--ignore-gpu-blocklist --enable-gpu-rasterization";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub action: String,
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub text: String,
}
impl Command {
    pub fn simple(action: &str) -> Self { Self { action: action.into(), id: 0, text: String::new() } }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub download_url: String,
    pub html_url: String,
}

#[derive(Debug)]
pub enum UserEvent {
    Command(Command),
    Title(u64, String),
    Source(u64, String),
    History(u64, bool, bool),
    Loading(u64, bool),
    Open(String),
    Suspended(u64, bool),
    DownloadStarted,
    Close,
    Favicon(u64, String),
    UpdateFound(UpdateInfo),
}

struct Tab {
    id: u64,
    view: Option<WebView>,
    title: String,
    url: String,
    favicon: String,
    back: bool,
    forward: bool,
    loading: bool,
    hidden_since: Option<Instant>,
    suspended: bool,
    suspending: bool,
    keep_awake: bool,
}

struct Browser {
    // Drop controllers before contexts/window. No callbacks hold a Browser reference.
    tabs: Vec<Tab>,
    chrome: WebView,
    context: WebContext,
    _chrome_context: WebContext,
    window: Window,
    proxy: EventLoopProxy<UserEvent>,
    downloads: Downloads,
    next_download: Rc<Cell<u64>>,
    blocking: Rc<Cell<bool>>,
    blocked: Rc<Cell<u64>>,
    active: u64,
    next_tab: u64,
    downloads_open: bool,
    ready: bool,
    notice: String,
    update: Option<UpdateInfo>,
    next_tick: Instant,
    last_snapshot: String,
    browser_args: &'static str,
}

impl Browser {
    fn new(window: Window, proxy: EventLoopProxy<UserEvent>) -> Result<Self> {
        let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
            .ok_or("Nie znaleziono folderu LOCALAPPDATA.")?.join("Fastium");
        std::fs::create_dir_all(&base)?;
        let force_gpu = std::env::args().any(|arg| arg == "--force-gpu");
        let browser_args = if force_gpu { FORCE_GPU_ARGS } else { BROWSER_ARGS };
        // Different engine arguments require distinct user-data directories.
        let base = if force_gpu { base.join("ForceGpu") } else { base };
        std::fs::create_dir_all(&base)?;
        // A separate local-UI profile prevents websites sharing its origin/storage.
        let mut chrome_context = WebContext::new(Some(base.join("Interface")));
        let ipc_proxy = proxy.clone();
        let chrome = WebViewBuilder::with_web_context(&mut chrome_context)
            .with_additional_browser_args(browser_args)
            .with_html(include_str!("../assets/shell.html"))
            .with_background_color((14, 17, 22, 255))
            .with_devtools(cfg!(debug_assertions))
            .with_navigation_handler(|url| url == "about:blank" || url.starts_with("data:text/html"))
            .with_new_window_req_handler(|_| false)
            .with_ipc_handler(move |request| {
                let uri = request.uri().to_string();
                if (uri != "about:blank" && !uri.starts_with("data:text/html")) || request.body().len() > 16384 { return; }
                if let Ok(cmd) = serde_json::from_str::<Command>(request.body()) {
                    let _ = ipc_proxy.send_event(UserEvent::Command(cmd));
                }
            })
            .build_as_child(&window)?;
        native::shortcuts(&chrome, proxy.clone())?;
        // UI geometry is expressed in DIP; Ctrl+wheel must not resize the chrome.
        unsafe {
            let settings = native::core(&chrome)?.Settings()?;
            settings.SetIsZoomControlEnabled(false)?;
            settings.SetAreDefaultContextMenusEnabled(false)?;
        }
        let context = WebContext::new(Some(base.join("Profile")));
        let mut browser = Self {
            tabs: Vec::new(), chrome, context, _chrome_context: chrome_context, window, proxy: proxy.clone(),
            downloads: Rc::new(RefCell::new(Vec::new())), next_download: Rc::new(Cell::new(1)),
            blocking: Rc::new(Cell::new(true)), blocked: Rc::new(Cell::new(0)),
            active: 0, next_tab: 1, downloads_open: false, ready: false, notice: String::new(),
            update: None,
            next_tick: Instant::now() + Duration::from_secs(2), last_snapshot: String::new(),
            browser_args,
        };
        check_github_update(proxy);
        browser.new_tab(policy::HOME)?;
        let size = browser.window.inner_size();
        browser.window.set_inner_size(tao::dpi::PhysicalSize::new(size.width + 1, size.height));
        browser.window.set_inner_size(size);
        browser.resize()?;
        Ok(browser)
    }

    fn new_tab(&mut self, url: &str) -> Result<()> {
        let id = self.next_tab;
        self.next_tab += 1;
        self.tabs.push(Tab { id, view: None, title: "Nowa karta".into(), url: policy::HOME.into(),
            favicon: String::new(),
            back: false, forward: false, loading: false, hidden_since: None, suspended: false,
            suspending: false, keep_awake: false });
        self.activate(id)?;
        if url != policy::HOME { self.navigate(url)?; }
        Ok(())
    }

    fn activate(&mut self, id: u64) -> Result<()> {
        if !self.tabs.iter().any(|t| t.id == id) { return Ok(()); }
        for tab in &mut self.tabs {
            if tab.id == id {
                if let Some(view) = &tab.view {
                    native::resume(view)?;
                    view.set_memory_usage_level(MemoryUsageLevel::Normal)?;
                    view.set_visible(true)?;
                    view.focus()?;
                } else { self.chrome.focus()?; }
                tab.hidden_since = None;
                tab.suspended = false;
            } else if tab.hidden_since.is_none() {
                if let Some(view) = &tab.view {
                    view.set_visible(false)?;
                    view.set_memory_usage_level(MemoryUsageLevel::Low)?;
                }
                tab.hidden_since = Some(Instant::now());
            }
        }
        self.active = id;
        self.notice.clear();
        self.resize()?;
        Ok(())
    }

    fn navigate(&mut self, input: &str) -> Result<()> {
        let url = policy::resolve(input)?;
        let i = self.tabs.iter().position(|t| t.id == self.active).ok_or("Brak aktywnej karty")?;
        if url == policy::HOME {
            if self.has_download(self.active) {
                return Err("Zakończ lub anuluj pobieranie tej karty przed przejściem na stronę startową.".into());
            }
            self.tabs[i].view = None;
            // Retire the view identity so queued events from its old controller cannot
            // overwrite a newly opened page or mark a local dashboard as suspended.
            self.tabs[i].id = self.next_tab;
            self.active = self.next_tab;
            self.next_tab += 1;
            self.tabs[i].url = url;
            self.tabs[i].title = "Nowa karta".into();
            self.tabs[i].favicon.clear();
            self.tabs[i].back = false;
            self.tabs[i].forward = false;
            self.tabs[i].loading = false;
            self.tabs[i].suspended = false;
            self.tabs[i].suspending = false;
            self.chrome.focus()?;
            return Ok(());
        }
        if self.tabs[i].view.is_none() {
            let id = self.active;
            let title_proxy = self.proxy.clone();
            let load_proxy = self.proxy.clone();
            let open_proxy = self.proxy.clone();
            // Create blank, attach network filters, then navigate: first request is filtered too.
            let view = WebViewBuilder::with_web_context(&mut self.context)
                .with_additional_browser_args(self.browser_args)
                .with_visible(false)
                .with_devtools(cfg!(debug_assertions))
                .with_navigation_handler(|url| policy::web_url(&url).is_some())
                .with_new_window_req_handler(move |url| {
                    if let Some(url) = policy::web_url(&url) { let _ = open_proxy.send_event(UserEvent::Open(url)); }
                    false
                })
                .with_document_title_changed_handler(move |title| {
                    let _ = title_proxy.send_event(UserEvent::Title(id, title));
                })
                .with_on_page_load_handler(move |event, _| {
                    let _ = load_proxy.send_event(UserEvent::Loading(id, matches!(event, PageLoadEvent::Started)));
                })
                .build_as_child(&self.window)?;
            native::attach(&view, id, self.proxy.clone(), self.blocking.clone(), self.blocked.clone(),
                self.downloads.clone(), self.next_download.clone())?;
            self.tabs[i].view = Some(view);
        }
        self.tabs[i].url = url.clone();
        self.tabs[i].loading = true;
        self.tabs[i].title = "Ładowanie…".into();
        self.resize()?;
        if let Some(view) = &self.tabs[i].view {
            native::resume(view)?;
            view.set_visible(true)?;
            view.load_url(&url)?;
            view.focus()?;
        }
        self.notice.clear();
        Ok(())
    }

    fn has_download(&self, tab: u64) -> bool {
        self.downloads.borrow().iter().any(|d| d.tab == tab && d.operation.is_some())
    }

    fn close_tab(&mut self, id: u64) -> Result<()> {
        if self.has_download(id) {
            return Err("Ta karta pobiera plik. Zakończ lub anuluj pobieranie przed jej zamknięciem.".into());
        }
        let Some(i) = self.tabs.iter().position(|t| t.id == id) else { return Ok(()) };
        self.tabs.remove(i); // wry closes controller; engine reclaims renderer resources.
        if self.tabs.is_empty() { self.new_tab(policy::HOME)?; }
        else if self.active == id { self.activate(self.tabs[i.min(self.tabs.len() - 1)].id)?; }
        Ok(())
    }

    fn command(&mut self, cmd: Command) -> Result<()> {
        match cmd.action.as_str() {
            "ready" => {
                self.ready = true;
                let size = self.window.inner_size();
                self.window.set_inner_size(tao::dpi::PhysicalSize::new(size.width + 1, size.height));
                self.window.set_inner_size(size);
                self.window.set_visible(true);
                self.resize()?;
            }
            "new" => { self.new_tab(policy::HOME)?; self.focus_address()?; }
            "navigate" => self.navigate(&cmd.text)?,
            "home" => self.navigate(policy::HOME)?,
            "switch" => self.activate(cmd.id)?,
            "close" => self.close_tab(cmd.id)?,
            "close_active" => self.close_tab(self.active)?,
            "focus" => self.focus_address()?,
            "downloads" => { self.downloads_open = !self.downloads_open; self.resize()?; }
            "block" => { self.blocking.set(!self.blocking.get()); self.notice = "Zmieniono ochronę. Odśwież stronę, aby zastosować zmianę do już załadowanych zasobów.".into(); }
            "awake" => {
                if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == self.active) { tab.keep_awake = !tab.keep_awake; }
            }
            "next_tab" | "previous_tab" => {
                if let Some(i) = self.tabs.iter().position(|t| t.id == self.active) {
                    let len = self.tabs.len();
                    let next = if cmd.action == "next_tab" { (i + 1) % len } else { (i + len - 1) % len };
                    self.activate(self.tabs[next].id)?;
                }
            }
            "back" | "forward" | "reload" | "stop" => {
                if let Some(view) = self.tabs.iter().find(|t| t.id == self.active).and_then(|t| t.view.as_ref()) {
                    let core = native::core(view)?;
                    // SAFETY: live COM interface used on its owning UI thread.
                    unsafe { match cmd.action.as_str() {
                        "back" => core.GoBack()?, "forward" => core.GoForward()?,
                        "reload" => core.Reload()?, _ => core.Stop()?,
                    } }
                }
            }
            "pause" | "resume" | "cancel" => {
                if let Some(download) = self.downloads.borrow_mut().iter_mut().find(|d| d.id == cmd.id) {
                    download.action(&cmd.action)?;
                }
            }
            "open_file" => {
                if let Some(download) = self.downloads.borrow().iter().find(|d| d.id == cmd.id) {
                    native::open_file(&download.path);
                }
            }
            "show_folder" => {
                if let Some(download) = self.downloads.borrow().iter().find(|d| d.id == cmd.id) {
                    native::show_in_folder(&download.path);
                }
            }
            "open_downloads_folder" => {
                native::open_downloads_folder();
            }
            "clear_download" => {
                let mut list = self.downloads.borrow_mut();
                if let Some(idx) = list.iter().position(|d| d.id == cmd.id && d.operation.is_none()) {
                    let d = list.remove(idx);
                    native::delete_file(&d.path);
                }
            }
            "clear_completed" => {
                let mut list = self.downloads.borrow_mut();
                list.retain(|d| d.operation.is_some());
            }
            "update_dismiss" => {
                self.update = None;
            }
            "update_now" => {
                if let Some(update) = self.update.take() {
                    self.notice = format!("Pobieranie instalatora Fastium v{}...", update.version);
                    let proxy = self.proxy.clone();
                    std::thread::spawn(move || {
                        let temp_dir = std::env::temp_dir();
                        let target_installer = temp_dir.join("FastiumSetup_Update.exe");
                        let status = std::process::Command::new("curl.exe")
                            .creation_flags(CREATE_NO_WINDOW)
                            .args(["-L", "-s", "-o", target_installer.to_str().unwrap_or(""), &update.download_url])
                            .status();

                        if let Ok(st) = status {
                            if st.success() && target_installer.exists() {
                                let _ = std::process::Command::new(&target_installer).spawn();
                                let _ = proxy.send_event(UserEvent::Close);
                                return;
                            }
                        }
                        let _ = std::process::Command::new("explorer.exe")
                            .arg(&update.html_url)
                            .spawn();
                    });
                }
            }
            "window_minimize" => { self.window.set_minimized(true); }
            "window_maximize" => { self.window.set_maximized(!self.window.is_maximized()); }
            "window_close" => { let _ = self.proxy.send_event(UserEvent::Close); }
            _ => {}
        }
        Ok(())
    }

    fn focus_address(&self) -> Result<()> {
        self.chrome.focus()?;
        self.chrome.evaluate_script("window.focusAddress()")?;
        Ok(())
    }

    fn resize(&self) -> Result<()> {
        let size = self.window.inner_size().to_logical::<f64>(self.window.scale_factor());
        self.chrome.set_bounds(Rect { position: LogicalPosition::new(0.0, 0.0).into(),
            size: LogicalSize::new(size.width.max(1.0), size.height.max(1.0)).into() })?;
        let bottom = FOOTER + if self.downloads_open { DOWNLOAD_PANEL } else { 0.0 };
        for tab in &self.tabs {
            if let Some(view) = &tab.view {
                view.set_bounds(Rect { position: LogicalPosition::new(0.0, TOOLBAR).into(),
                    size: LogicalSize::new(size.width.max(1.0), (size.height - TOOLBAR - bottom).max(1.0)).into() })?;
            }
        }
        Ok(())
    }

    fn event(&mut self, event: UserEvent) -> Result<()> {
        match event {
            UserEvent::Command(cmd) => self.command(cmd)?,
            UserEvent::Open(url) => self.new_tab(&url)?,
            UserEvent::DownloadStarted => {
                self.downloads_open = true;
                self.next_tick = Instant::now();
                self.resize()?;
            }
            UserEvent::Title(id, title) => {
                if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id && t.view.is_some()) {
                    t.title = title.chars().take(256).collect();
                }
            }
            UserEvent::Source(id, url) => {
                if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id && t.view.is_some()) {
                    if policy::web_url(&url).is_some() { t.url = url; }
                }
            }
            UserEvent::History(id, back, forward) => {
                if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id && t.view.is_some()) {
                    t.back = back; t.forward = forward;
                }
            }
            UserEvent::Loading(id, loading) => {
                if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id && t.view.is_some()) { t.loading = loading; }
            }
            UserEvent::Suspended(id, success) => {
                if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id) {
                    t.suspending = false;
                    if id == self.active || t.keep_awake {
                        // A late completion must never leave a newly active tab suspended.
                        if let Some(view) = &t.view { native::resume(view)?; }
                        t.suspended = false;
                    } else { t.suspended = success; }
                }
            }
            UserEvent::Close => {}
            UserEvent::Favicon(id, favicon) => {
                if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id && t.view.is_some()) {
                    t.favicon = favicon;
                }
            }
            UserEvent::UpdateFound(info) => {
                self.update = Some(info);
            }
        }
        Ok(())
    }

    fn tick(&mut self) -> Result<()> {
        if !self.ready {
            self.ready = true;
            self.window.set_visible(true);
            let _ = self.resize();
        }
        let mut has_operations = false;
        {
            let mut downloads = self.downloads.borrow_mut();
            for d in downloads.iter_mut() {
                if let Err(error) = d.refresh() { self.notice = format!("Odczyt pobierania: {error}"); }
                has_operations |= d.operation.is_some();
            }
        }
        self.next_tick = Instant::now() + if has_operations { Duration::from_millis(500) } else { Duration::from_secs(2) };
        for tab in &mut self.tabs {
            if tab.id == self.active || tab.keep_awake || tab.suspended || tab.suspending { continue; }
            if !tab.hidden_since.is_some_and(|t| t.elapsed() >= SLEEP_AFTER) { continue; }
            if self.downloads.borrow().iter().any(|d| d.tab == tab.id && d.operation.is_some()) { continue; }
            if let Some(view) = &tab.view {
                if !native::playing_audio(view) {
                    native::suspend(view, tab.id, self.proxy.clone())?;
                    tab.suspending = true;
                }
            }
        }
        Ok(())
    }

    fn render(&mut self) -> Result<()> {
        if !self.ready { return Ok(()); }
        let tabs: Vec<_> = self.tabs.iter().map(|t| json!({
            "id": t.id, "title": t.title, "url": t.url, "home": t.view.is_none(),
            "back": t.back, "forward": t.forward, "loading": t.loading,
            "sleeping": t.suspended, "awake": t.keep_awake,
            "favicon": t.favicon,
        })).collect();
        let snapshot = json!({ "tabs": tabs, "active": self.active,
            "blocking": self.blocking.get(), "blocked": self.blocked.get(),
            "downloadsOpen": self.downloads_open, "downloads": *self.downloads.borrow(),
            "notice": self.notice, "update": self.update }).to_string();
        if snapshot != self.last_snapshot {
            // JSON serialization + textContent in shell: page titles/URLs never become HTML/JS.
            self.chrome.evaluate_script(&format!("window.render({snapshot})"))?;
            self.last_snapshot = snapshot;
        }
        Ok(())
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        // Release operation references before controllers, including interrupted downloads.
        for download in self.downloads.borrow_mut().iter_mut() { let _ = download.action("cancel"); }
        self.downloads.borrow_mut().clear();
    }
}

pub fn run() -> Result<()> {
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let mut builder = WindowBuilder::new().with_title("Fastium")
        .with_decorations(false)
        .with_undecorated_shadow(true)
        .with_visible(false)
        .with_inner_size(tao::dpi::LogicalSize::new(1180.0, 780.0))
        .with_min_inner_size(tao::dpi::LogicalSize::new(640.0, 480.0));
    if let Some(icon) = native::app_icon() {
        builder = builder.with_window_icon(Some(icon));
    }
    let window = builder.build(&event_loop)?;
    native::setup_window(&window);
    let mut browser = Browser::new(window, event_loop.create_proxy())?;
    // Return normally so Browser::drop releases downloads and controllers before exit.
    event_loop.run_return(|event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(browser.next_tick);
        let result = match event {
            Event::UserEvent(UserEvent::Close)
            | Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                *control_flow = ControlFlow::Exit;
                Ok(())
            }
            Event::UserEvent(event) => browser.event(event),
            Event::WindowEvent { event: WindowEvent::Resized(_), .. }
            | Event::WindowEvent { event: WindowEvent::ScaleFactorChanged { .. }, .. } => browser.resize(),
            Event::MainEventsCleared => {
                if Instant::now() >= browser.next_tick { browser.tick() } else { Ok(()) }
            }
            _ => Ok(()),
        };
        if let Err(error) = result { browser.notice = error.to_string(); }
        if let Err(error) = browser.render() { eprintln!("Fastium UI: {error}"); }
    });
    Ok(())
}

fn is_newer_version(remote: &str, current: &str) -> bool {
    let clean = |s: &str| -> Vec<u32> {
        s.trim_start_matches(|c: char| c == 'v' || c == 'V')
            .split('.')
            .filter_map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok())
            .collect()
    };
    let r = clean(remote);
    let c = clean(current);
    for (rv, cv) in r.iter().zip(c.iter()) {
        if rv > cv { return true; }
        if rv < cv { return false; }
    }
    r.len() > c.len()
}

fn check_github_update(proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        // Opóźnienie 3 sekund, aby start przeglądarki był natychmiastowy i płynny
        std::thread::sleep(Duration::from_secs(3));

        let output = std::process::Command::new("curl.exe")
            .creation_flags(CREATE_NO_WINDOW)
            .args(["-s", "-H", "User-Agent: Fastium", "https://api.github.com/repos/Miko881/fastium/releases/latest"])
            .output();

        let Ok(output) = output else { return };
        if !output.status.success() { return; }

        let Ok(val) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else { return };
        let Some(tag) = val.get("tag_name").and_then(|t| t.as_str()) else { return };
        let html_url = val.get("html_url").and_then(|u| u.as_str()).unwrap_or("https://github.com/Miko881/fastium/releases/latest").to_string();

        let current_version = env!("CARGO_PKG_VERSION");
        if is_newer_version(tag, current_version) {
            let mut download_url = html_url.clone();
            if let Some(assets) = val.get("assets").and_then(|a| a.as_array()) {
                for asset in assets {
                    let name = asset.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    if name.eq_ignore_ascii_case("FastiumSetup.exe") {
                        if let Some(url) = asset.get("browser_download_url").and_then(|u| u.as_str()) {
                            download_url = url.to_string();
                            break;
                        }
                    }
                }
            }
            let _ = proxy.send_event(UserEvent::UpdateFound(UpdateInfo {
                version: tag.trim_start_matches(|c| c == 'v' || c == 'V').to_string(),
                download_url,
                html_url,
            }));
        }
    });
}
