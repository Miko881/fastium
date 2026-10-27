use std::{cell::{Cell, RefCell}, error::Error, os::windows::process::CommandExt, path::PathBuf, rc::Rc, time::{Duration, Instant}};
use serde::Deserialize;
use serde_json::json;
use tao::{event::{Event, WindowEvent}, event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget}, window::{Window, WindowBuilder}};
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
const BROWSER_ARGS: &str = "";
const FORCE_GPU_ARGS: &str = "--ignore-gpu-blocklist --enable-gpu-rasterization";

#[derive(Debug, Deserialize)]
pub struct Command {
    pub action: String,
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
}
impl Command {
    pub fn simple(action: &str) -> Self { Self { action: action.into(), id: 0, text: String::new(), x: 0, y: 0 } }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub download_url: String,
    pub html_url: String,
}

#[derive(Debug)]
pub enum UserEvent {
    Command(Option<tao::window::WindowId>, Command),
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

struct WindowState {
    id: tao::window::WindowId,
    window: Window,
    chrome: WebView,
    tabs: Vec<Tab>,
    active: u64,
    downloads_open: bool,
    ready: bool,
    last_snapshot: String,
}

impl WindowState {
    fn resize(&self) -> Result<()> {
        let size = self.window.inner_size().to_logical::<f64>(self.window.scale_factor());
        self.chrome.set_bounds(Rect {
            position: LogicalPosition::new(0.0, 0.0).into(),
            size: LogicalSize::new(size.width.max(1.0), size.height.max(1.0)).into(),
        })?;
        let bottom = FOOTER + if self.downloads_open { DOWNLOAD_PANEL } else { 0.0 };
        for tab in &self.tabs {
            if let Some(view) = &tab.view {
                view.set_bounds(Rect {
                    position: LogicalPosition::new(0.0, TOOLBAR).into(),
                    size: LogicalSize::new(size.width.max(1.0), (size.height - TOOLBAR - bottom).max(1.0)).into(),
                })?;
            }
        }
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
                } else {
                    self.chrome.focus()?;
                }
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
        self.resize()?;
        Ok(())
    }

    fn focus_address(&self) -> Result<()> {
        self.chrome.focus()?;
        self.chrome.evaluate_script("window.focusAddress()")?;
        Ok(())
    }

    fn render(
        &mut self,
        notice: &str,
        update: &Option<UpdateInfo>,
        downloads: &Downloads,
        blocking: bool,
        blocked: u64,
    ) -> Result<()> {
        if !self.ready { return Ok(()); }
        let tabs: Vec<_> = self.tabs.iter().map(|t| json!({
            "id": t.id, "title": t.title, "url": t.url, "home": t.view.is_none(),
            "back": t.back, "forward": t.forward, "loading": t.loading,
            "sleeping": t.suspended, "awake": t.keep_awake,
            "favicon": t.favicon,
        })).collect();
        let snapshot = json!({
            "tabs": tabs,
            "active": self.active,
            "blocking": blocking,
            "blocked": blocked,
            "downloadsOpen": self.downloads_open,
            "downloads": *downloads.borrow(),
            "notice": notice,
            "update": update,
            "version": env!("CARGO_PKG_VERSION"),
        }).to_string();
        if snapshot != self.last_snapshot {
            self.chrome.evaluate_script(&format!("window.render({snapshot})"))?;
            self.last_snapshot = snapshot;
        }
        Ok(())
    }
}

struct Browser {
    windows: Vec<WindowState>,
    context: WebContext,
    chrome_context: WebContext,
    proxy: EventLoopProxy<UserEvent>,
    downloads: Downloads,
    next_download: Rc<Cell<u64>>,
    blocking: Rc<Cell<bool>>,
    blocked: Rc<Cell<u64>>,
    next_tab: u64,
    notice: String,
    update: Option<UpdateInfo>,
    next_tick: Instant,
    browser_args: &'static str,
}

impl Browser {
    fn new(
        target: &EventLoopWindowTarget<UserEvent>,
        initial_pos: Option<(i32, i32)>,
        proxy: EventLoopProxy<UserEvent>,
    ) -> Result<Self> {
        let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
            .ok_or("Nie znaleziono folderu LOCALAPPDATA.")?.join("Fastium");
        std::fs::create_dir_all(&base)?;
        let force_gpu = std::env::args().any(|arg| arg == "--force-gpu");
        let browser_args = if force_gpu { FORCE_GPU_ARGS } else { BROWSER_ARGS };
        let base = if force_gpu { base.join("ForceGpu") } else { base };
        std::fs::create_dir_all(&base)?;
        let chrome_context = WebContext::new(Some(base.join("Interface")));
        let context = WebContext::new(Some(base.join("Profile")));

        let mut browser = Self {
            windows: Vec::new(),
            context,
            chrome_context,
            proxy: proxy.clone(),
            downloads: Rc::new(RefCell::new(Vec::new())),
            next_download: Rc::new(Cell::new(1)),
            blocking: Rc::new(Cell::new(true)),
            blocked: Rc::new(Cell::new(0)),
            next_tab: 1,
            notice: String::new(),
            update: None,
            next_tick: Instant::now() + Duration::from_secs(2),
            browser_args,
        };

        let initial_tab_id = browser.next_tab;
        browser.next_tab += 1;
        let initial_tab = Tab {
            id: initial_tab_id,
            view: None,
            title: "Nowa karta".into(),
            url: policy::HOME.into(),
            favicon: String::new(),
            back: false,
            forward: false,
            loading: false,
            hidden_since: None,
            suspended: false,
            suspending: false,
            keep_awake: false,
        };

        let mut builder = WindowBuilder::new()
            .with_title("Fastium")
            .with_decorations(false)
            .with_undecorated_shadow(true)
            .with_visible(false)
            .with_inner_size(tao::dpi::LogicalSize::new(1180.0, 780.0))
            .with_min_inner_size(tao::dpi::LogicalSize::new(640.0, 480.0));
        if let Some((x, y)) = initial_pos {
            builder = builder.with_position(tao::dpi::PhysicalPosition::new(x, y));
        }
        if let Some(icon) = native::app_icon() {
            builder = builder.with_window_icon(Some(icon));
        }
        let window = builder.build(target)?;
        native::setup_window(&window);

        let win_id = window.id();
        let ipc_proxy = proxy.clone();
        let chrome = WebViewBuilder::with_web_context(&mut browser.chrome_context)
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
                    let _ = ipc_proxy.send_event(UserEvent::Command(Some(win_id), cmd));
                }
            })
            .build_as_child(&window)?;

        native::shortcuts(&chrome, proxy.clone())?;
        unsafe {
            let settings = native::core(&chrome)?.Settings()?;
            settings.SetIsZoomControlEnabled(false)?;
            settings.SetAreDefaultContextMenusEnabled(false)?;
        }

        let mut win_state = WindowState {
            id: win_id,
            window,
            chrome,
            tabs: vec![initial_tab],
            active: initial_tab_id,
            downloads_open: false,
            ready: false,
            last_snapshot: String::new(),
        };
        win_state.activate(initial_tab_id)?;
        let size = win_state.window.inner_size();
        win_state.window.set_inner_size(tao::dpi::PhysicalSize::new(size.width + 1, size.height));
        win_state.window.set_inner_size(size);
        win_state.resize()?;
        browser.windows.push(win_state);

        check_github_update(proxy);

        let initial_url = std::env::args()
            .skip(1)
            .find(|arg| !arg.starts_with("--"))
            .unwrap_or_else(|| policy::HOME.to_string());
        if initial_url != policy::HOME {
            browser.navigate(0, &initial_url)?;
        }

        Ok(browser)
    }

    fn create_tab_view(&mut self, win_idx: usize, tab_id: u64) -> Result<WebView> {
        let title_proxy = self.proxy.clone();
        let load_proxy = self.proxy.clone();
        let open_proxy = self.proxy.clone();
        let window = &self.windows[win_idx].window;
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
                let _ = title_proxy.send_event(UserEvent::Title(tab_id, title));
            })
            .with_on_page_load_handler(move |event, _| {
                let _ = load_proxy.send_event(UserEvent::Loading(tab_id, matches!(event, PageLoadEvent::Started)));
            })
            .build_as_child(window)?;
        native::attach(&view, tab_id, self.proxy.clone(), self.blocking.clone(), self.blocked.clone(),
            self.downloads.clone(), self.next_download.clone())?;
        Ok(view)
    }

    fn create_window(
        &mut self,
        target: &EventLoopWindowTarget<UserEvent>,
        x: i32,
        y: i32,
        tab: Tab,
    ) -> Result<WindowState> {
        let mut builder = WindowBuilder::new()
            .with_title("Fastium")
            .with_decorations(false)
            .with_undecorated_shadow(true)
            .with_visible(false)
            .with_inner_size(tao::dpi::LogicalSize::new(1180.0, 780.0))
            .with_min_inner_size(tao::dpi::LogicalSize::new(640.0, 480.0))
            .with_position(tao::dpi::PhysicalPosition::new(x, y));
        if let Some(icon) = native::app_icon() {
            builder = builder.with_window_icon(Some(icon));
        }
        let window = builder.build(target)?;
        native::setup_window(&window);

        let win_id = window.id();
        let ipc_proxy = self.proxy.clone();
        let chrome = WebViewBuilder::with_web_context(&mut self.chrome_context)
            .with_additional_browser_args(self.browser_args)
            .with_html(include_str!("../assets/shell.html"))
            .with_background_color((14, 17, 22, 255))
            .with_devtools(cfg!(debug_assertions))
            .with_navigation_handler(|url| url == "about:blank" || url.starts_with("data:text/html"))
            .with_new_window_req_handler(|_| false)
            .with_ipc_handler(move |request| {
                let uri = request.uri().to_string();
                if (uri != "about:blank" && !uri.starts_with("data:text/html")) || request.body().len() > 16384 { return; }
                if let Ok(cmd) = serde_json::from_str::<Command>(request.body()) {
                    let _ = ipc_proxy.send_event(UserEvent::Command(Some(win_id), cmd));
                }
            })
            .build_as_child(&window)?;

        native::shortcuts(&chrome, self.proxy.clone())?;
        unsafe {
            let settings = native::core(&chrome)?.Settings()?;
            settings.SetIsZoomControlEnabled(false)?;
            settings.SetAreDefaultContextMenusEnabled(false)?;
        }

        if let Some(view) = &tab.view {
            native::reparent_view(view, &window)?;
        }

        let tab_id = tab.id;
        let mut win_state = WindowState {
            id: win_id,
            window,
            chrome,
            tabs: vec![tab],
            active: tab_id,
            downloads_open: false,
            ready: false,
            last_snapshot: String::new(),
        };
        win_state.activate(tab_id)?;
        let size = win_state.window.inner_size();
        win_state.window.set_inner_size(tao::dpi::PhysicalSize::new(size.width + 1, size.height));
        win_state.window.set_inner_size(size);
        win_state.resize()?;
        win_state.ready = true;
        win_state.window.set_visible(true);
        let _ = win_state.render(&self.notice, &self.update, &self.downloads, self.blocking.get(), self.blocked.get());
        Ok(win_state)
    }

    fn new_tab(&mut self, win_idx: usize, url: &str) -> Result<()> {
        let id = self.next_tab;
        self.next_tab += 1;
        self.windows[win_idx].tabs.push(Tab {
            id, view: None, title: "Nowa karta".into(), url: policy::HOME.into(),
            favicon: String::new(),
            back: false, forward: false, loading: false, hidden_since: None, suspended: false,
            suspending: false, keep_awake: false,
        });
        self.windows[win_idx].activate(id)?;
        if url != policy::HOME {
            self.navigate(win_idx, url)?;
        }
        Ok(())
    }

    fn navigate(&mut self, win_idx: usize, input: &str) -> Result<()> {
        let url = policy::resolve(input)?;
        let active_id = self.windows[win_idx].active;
        let i = self.windows[win_idx].tabs.iter().position(|t| t.id == active_id).ok_or("Brak aktywnej karty")?;
        if url == policy::HOME {
            if self.has_download(active_id) {
                return Err("Zakończ lub anuluj pobieranie tej karty przed przejściem na stronę startową.".into());
            }
            self.windows[win_idx].tabs[i].view = None;
            self.windows[win_idx].tabs[i].id = self.next_tab;
            self.windows[win_idx].active = self.next_tab;
            self.next_tab += 1;
            self.windows[win_idx].tabs[i].url = url;
            self.windows[win_idx].tabs[i].title = "Nowa karta".into();
            self.windows[win_idx].tabs[i].favicon.clear();
            self.windows[win_idx].tabs[i].back = false;
            self.windows[win_idx].tabs[i].forward = false;
            self.windows[win_idx].tabs[i].loading = false;
            self.windows[win_idx].tabs[i].suspended = false;
            self.windows[win_idx].tabs[i].suspending = false;
            self.windows[win_idx].chrome.focus()?;
            return Ok(());
        }
        if self.windows[win_idx].tabs[i].view.is_none() {
            let view = self.create_tab_view(win_idx, active_id)?;
            self.windows[win_idx].tabs[i].view = Some(view);
        }
        self.windows[win_idx].tabs[i].url = url.clone();
        self.windows[win_idx].tabs[i].loading = true;
        self.windows[win_idx].tabs[i].title = "Ładowanie…".into();
        self.windows[win_idx].resize()?;
        if let Some(view) = &self.windows[win_idx].tabs[i].view {
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

    fn close_tab(&mut self, win_idx: usize, id: u64) -> Result<()> {
        if self.has_download(id) {
            return Err("Ta karta pobiera plik. Zakończ lub anuluj pobieranie przed jej zamknięciem.".into());
        }
        let Some(i) = self.windows[win_idx].tabs.iter().position(|t| t.id == id) else { return Ok(()) };
        self.windows[win_idx].tabs.remove(i);
        if self.windows[win_idx].tabs.is_empty() {
            if self.windows.len() > 1 {
                self.windows.remove(win_idx);
            } else {
                self.new_tab(win_idx, policy::HOME)?;
            }
        } else if self.windows[win_idx].active == id {
            let next_id = self.windows[win_idx].tabs[i.min(self.windows[win_idx].tabs.len() - 1)].id;
            self.windows[win_idx].activate(next_id)?;
        }
        Ok(())
    }

    fn close_window(&mut self, window_id: tao::window::WindowId) {
        if let Some(pos) = self.windows.iter().position(|w| w.id == window_id) {
            self.windows.remove(pos);
        }
    }

    fn command(
        &mut self,
        win_id: Option<tao::window::WindowId>,
        cmd: Command,
        target: &EventLoopWindowTarget<UserEvent>,
    ) -> Result<()> {
        if self.windows.is_empty() { return Ok(()); }

        let win_idx = if let Some(wid) = win_id {
            self.windows.iter().position(|w| w.id == wid).unwrap_or(0)
        } else if cmd.id != 0 {
            self.windows.iter().position(|w| w.tabs.iter().any(|t| t.id == cmd.id)).unwrap_or(0)
        } else {
            use tao::platform::windows::WindowExtWindows;
            let fg = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
            self.windows.iter().position(|w| windows::Win32::Foundation::HWND(w.window.hwnd() as _) == fg).unwrap_or(0)
        };

        match cmd.action.as_str() {
            "ready" => {
                let win = &mut self.windows[win_idx];
                win.ready = true;
                let size = win.window.inner_size();
                win.window.set_inner_size(tao::dpi::PhysicalSize::new(size.width + 1, size.height));
                win.window.set_inner_size(size);
                win.window.set_visible(true);
                win.resize()?;
            }
            "new" => {
                self.new_tab(win_idx, policy::HOME)?;
                self.windows[win_idx].focus_address()?;
            }
            "navigate" => self.navigate(win_idx, &cmd.text)?,
            "home" => self.navigate(win_idx, policy::HOME)?,
            "switch" => self.windows[win_idx].activate(cmd.id)?,
            "close" => self.close_tab(win_idx, cmd.id)?,
            "close_active" => {
                let active = self.windows[win_idx].active;
                self.close_tab(win_idx, active)?;
            }
            "focus" => self.windows[win_idx].focus_address()?,
            "downloads" => {
                self.windows[win_idx].downloads_open = !self.windows[win_idx].downloads_open;
                self.windows[win_idx].resize()?;
            }
            "block" => {
                self.blocking.set(!self.blocking.get());
                self.notice = "Zmieniono ochronę. Odśwież stronę, aby zastosować zmianę do już załadowanych zasobów.".into();
            }
            "awake" => {
                let active = self.windows[win_idx].active;
                if let Some(tab) = self.windows[win_idx].tabs.iter_mut().find(|t| t.id == active) {
                    tab.keep_awake = !tab.keep_awake;
                }
            }
            "next_tab" | "previous_tab" => {
                let active = self.windows[win_idx].active;
                if let Some(i) = self.windows[win_idx].tabs.iter().position(|t| t.id == active) {
                    let len = self.windows[win_idx].tabs.len();
                    let next = if cmd.action == "next_tab" { (i + 1) % len } else { (i + len - 1) % len };
                    let next_id = self.windows[win_idx].tabs[next].id;
                    self.windows[win_idx].activate(next_id)?;
                }
            }
            "back" | "forward" | "reload" | "stop" => {
                let active = self.windows[win_idx].active;
                if let Some(view) = self.windows[win_idx].tabs.iter().find(|t| t.id == active).and_then(|t| t.view.as_ref()) {
                    let core = native::core(view)?;
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
            "window_minimize" => { self.windows[win_idx].window.set_minimized(true); }
            "window_maximize" => {
                let is_max = self.windows[win_idx].window.is_maximized();
                self.windows[win_idx].window.set_maximized(!is_max);
            }
            "window_close" => {
                if self.windows.len() > 1 {
                    self.windows.remove(win_idx);
                } else {
                    let _ = self.proxy.send_event(UserEvent::Close);
                }
            }
            "detach_tab" => {
                let Some(source_win_idx) = self.windows.iter().position(|w| w.tabs.iter().any(|t| t.id == cmd.id)) else { return Ok(()); };
                let tab_pos = self.windows[source_win_idx].tabs.iter().position(|t| t.id == cmd.id).unwrap();

                let mut cursor = windows::Win32::Foundation::POINT::default();
                let (target_x, target_y) = unsafe {
                    if windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut cursor).is_ok() {
                        (cursor.x, cursor.y)
                    } else {
                        (cmd.x, cmd.y)
                    }
                };

                // Karta jest jedyna w oknie: przesuń okno na docelowy monitor/pozycję bez przeładowywania i bez utraty stanu wideo
                if self.windows[source_win_idx].tabs.len() <= 1 {
                    let win = &mut self.windows[source_win_idx];
                    if win.window.is_maximized() {
                        win.window.set_maximized(false);
                    }
                    let spawn_x = target_x - 120;
                    let spawn_y = (target_y - 18).max(0);
                    win.window.set_outer_position(tao::dpi::PhysicalPosition::new(spawn_x, spawn_y));
                    let _ = win.resize();
                    return Ok(());
                }

                // Sprawdź czy karta została upuszczona na pasek innego otwartego okna Fastium
                let mut dropped_on_other = None;
                for (i, w) in self.windows.iter().enumerate() {
                    if i == source_win_idx { continue; }
                    if let Ok(pos) = w.window.outer_position() {
                        let size = w.window.inner_size();
                        let rect_left = pos.x;
                        let rect_right = pos.x + size.width as i32;
                        let rect_top = pos.y;
                        let rect_bottom = pos.y + 60;
                        if target_x >= rect_left && target_x <= rect_right && target_y >= rect_top && target_y <= rect_bottom {
                            dropped_on_other = Some(i);
                            break;
                        }
                    }
                }

                if let Some(dest_idx) = dropped_on_other {
                    let tab = self.windows[source_win_idx].tabs.remove(tab_pos);
                    if self.windows[source_win_idx].active == cmd.id {
                        let next_id = self.windows[source_win_idx].tabs[tab_pos.min(self.windows[source_win_idx].tabs.len() - 1)].id;
                        self.windows[source_win_idx].activate(next_id)?;
                    }
                    self.windows[source_win_idx].resize()?;

                    if let Some(view) = &tab.view {
                        native::reparent_view(view, &self.windows[dest_idx].window)?;
                    }
                    let tab_id = tab.id;
                    self.windows[dest_idx].tabs.push(tab);
                    self.windows[dest_idx].activate(tab_id)?;
                    self.windows[dest_idx].resize()?;
                    return Ok(());
                }

                // W oknie jest więcej kart: odrywamy kartę do nowego okna w tym samym procesie!
                // Dzięki temu WebView jest przepinane natywnie, wideo i strona NIE są przeładowywane!
                let tab = self.windows[source_win_idx].tabs.remove(tab_pos);
                if self.windows[source_win_idx].active == cmd.id {
                    let next_id = self.windows[source_win_idx].tabs[tab_pos.min(self.windows[source_win_idx].tabs.len() - 1)].id;
                    self.windows[source_win_idx].activate(next_id)?;
                }
                self.windows[source_win_idx].resize()?;

                let spawn_x = target_x - 120;
                let spawn_y = (target_y - 18).max(0);

                let new_win = self.create_window(target, spawn_x, spawn_y, tab)?;
                self.windows.push(new_win);
            }
            "reorder_tabs" => {
                let ids: Vec<u64> = cmd.text.split(',').filter_map(|s| s.parse().ok()).collect();
                let win = &mut self.windows[win_idx];
                if ids.len() == win.tabs.len() {
                    win.tabs.sort_by_key(|t| ids.iter().position(|&id| id == t.id).unwrap_or(usize::MAX));
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn event(&mut self, event: UserEvent, target: &EventLoopWindowTarget<UserEvent>) -> Result<()> {
        match event {
            UserEvent::Command(win_id, cmd) => self.command(win_id, cmd, target)?,
            UserEvent::Open(url) => {
                if !self.windows.is_empty() {
                    let idx = self.windows.len() - 1;
                    self.new_tab(idx, &url)?;
                }
            }
            UserEvent::DownloadStarted => {
                for win in &mut self.windows {
                    win.downloads_open = true;
                    let _ = win.resize();
                }
                self.next_tick = Instant::now();
            }
            UserEvent::Title(id, title) => {
                for win in &mut self.windows {
                    if let Some(t) = win.tabs.iter_mut().find(|t| t.id == id && t.view.is_some()) {
                        t.title = title.chars().take(256).collect();
                        break;
                    }
                }
            }
            UserEvent::Source(id, url) => {
                for win in &mut self.windows {
                    if let Some(t) = win.tabs.iter_mut().find(|t| t.id == id && t.view.is_some()) {
                        if policy::web_url(&url).is_some() { t.url = url; }
                        break;
                    }
                }
            }
            UserEvent::History(id, back, forward) => {
                for win in &mut self.windows {
                    if let Some(t) = win.tabs.iter_mut().find(|t| t.id == id && t.view.is_some()) {
                        t.back = back; t.forward = forward;
                        break;
                    }
                }
            }
            UserEvent::Loading(id, loading) => {
                for win in &mut self.windows {
                    if let Some(t) = win.tabs.iter_mut().find(|t| t.id == id && t.view.is_some()) {
                        t.loading = loading;
                        break;
                    }
                }
            }
            UserEvent::Suspended(id, success) => {
                for win in &mut self.windows {
                    if let Some(t) = win.tabs.iter_mut().find(|t| t.id == id) {
                        t.suspending = false;
                        if id == win.active || t.keep_awake {
                            if let Some(view) = &t.view { native::resume(view)?; }
                            t.suspended = false;
                        } else {
                            t.suspended = success;
                        }
                        break;
                    }
                }
            }
            UserEvent::Close => {}
            UserEvent::Favicon(id, favicon) => {
                for win in &mut self.windows {
                    if let Some(t) = win.tabs.iter_mut().find(|t| t.id == id && t.view.is_some()) {
                        t.favicon = favicon;
                        break;
                    }
                }
            }
            UserEvent::UpdateFound(info) => {
                self.update = Some(info);
            }
        }
        Ok(())
    }

    fn tick(&mut self) -> Result<()> {
        for win in &mut self.windows {
            if !win.ready {
                win.ready = true;
                win.window.set_visible(true);
                let _ = win.resize();
            }
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
        for win in &mut self.windows {
            for tab in &mut win.tabs {
                if tab.id == win.active || tab.keep_awake || tab.suspended || tab.suspending { continue; }
                if !tab.hidden_since.is_some_and(|t| t.elapsed() >= SLEEP_AFTER) { continue; }
                if self.downloads.borrow().iter().any(|d| d.tab == tab.id && d.operation.is_some()) { continue; }
                if let Some(view) = &tab.view {
                    if !native::playing_audio(view) {
                        native::suspend(view, tab.id, self.proxy.clone())?;
                        tab.suspending = true;
                    }
                }
            }
        }
        Ok(())
    }

    fn render(&mut self) -> Result<()> {
        for win in &mut self.windows {
            win.render(&self.notice, &self.update, &self.downloads, self.blocking.get(), self.blocked.get())?;
        }
        Ok(())
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        for download in self.downloads.borrow_mut().iter_mut() { let _ = download.action("cancel"); }
        self.downloads.borrow_mut().clear();
    }
}

pub fn run() -> Result<()> {
    let mut initial_pos = None;
    for arg in std::env::args() {
        if let Some(pos) = arg.strip_prefix("--window-pos=") {
            let parts: Vec<&str> = pos.split(',').collect();
            if parts.len() == 2 {
                if let (Ok(x), Ok(y)) = (parts[0].parse::<i32>(), parts[1].parse::<i32>()) {
                    initial_pos = Some((x, y));
                }
            }
        }
    }

    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let mut browser = Browser::new(&event_loop, initial_pos, event_loop.create_proxy())?;

    event_loop.run_return(|event, target, control_flow| {
        *control_flow = ControlFlow::WaitUntil(browser.next_tick);
        let result = match event {
            Event::UserEvent(UserEvent::Close) => {
                *control_flow = ControlFlow::Exit;
                Ok(())
            }
            Event::WindowEvent { window_id, event: WindowEvent::CloseRequested, .. } => {
                browser.close_window(window_id);
                if browser.windows.is_empty() {
                    *control_flow = ControlFlow::Exit;
                }
                Ok(())
            }
            Event::UserEvent(event) => browser.event(event, target),
            Event::WindowEvent { window_id, event: WindowEvent::Resized(_), .. }
            | Event::WindowEvent { window_id, event: WindowEvent::ScaleFactorChanged { .. }, .. } => {
                if let Some(win) = browser.windows.iter().find(|w| w.id == window_id) {
                    win.resize()
                } else {
                    Ok(())
                }
            }
            Event::MainEventsCleared => {
                if Instant::now() >= browser.next_tick { browser.tick() } else { Ok(()) }
            }
            _ => Ok(()),
        };
        if let Err(error) = result { browser.notice = error.to_string(); }
        if let Err(error) = browser.render() { eprintln!("Fastium UI: {error}"); }
        if browser.windows.is_empty() {
            *control_flow = ControlFlow::Exit;
        }
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
        std::thread::sleep(Duration::from_secs(3));

        loop {
            let output = std::process::Command::new("curl.exe")
                .creation_flags(CREATE_NO_WINDOW)
                .args(["-s", "-H", "User-Agent: Fastium", "https://api.github.com/repos/Miko881/fastium/releases/latest"])
                .output();

            if let Ok(output) = output {
                if output.status.success() {
                    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                        if let Some(tag) = val.get("tag_name").and_then(|t| t.as_str()) {
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
                                let _ = proxy.send_event(UserEvent::Command(None, Command::simple("update_found")));
                                let _ = proxy.send_event(UserEvent::UpdateFound(UpdateInfo {
                                    version: tag.trim_start_matches(|c| c == 'v' || c == 'V').to_string(),
                                    download_url,
                                    html_url,
                                }));
                                std::thread::sleep(Duration::from_secs(4 * 3600));
                                continue;
                            }
                        }
                    }
                }
            }

            std::thread::sleep(Duration::from_secs(3600));
        }
    });
}
