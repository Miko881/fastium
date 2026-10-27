//! WebView2 COM stays on the UI STA thread. No raw pointers cross threads.
use std::{cell::{Cell, RefCell}, rc::Rc, time::Instant};
use serde::Serialize;
use tao::event_loop::EventLoopProxy;
use webview2_com::{Microsoft::Web::WebView2::Win32::*, *};
use windows::{core::{w, Interface, PWSTR}, Win32::{Foundation::BOOL, UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_SHIFT}}};
use wry::{WebView, WebViewExtWindows};
use crate::{browser::{Command, UserEvent}, policy};

pub type Downloads = Rc<RefCell<Vec<Download>>>;

#[derive(Serialize)]
pub struct Download {
    pub id: u64,
    pub tab: u64,
    pub path: String,
    pub received: i64,
    pub total: i64,
    pub speed: i64,
    pub state: String,
    pub resumable: bool,
    pub reason: i32,
    #[serde(skip)]
    pub operation: Option<ICoreWebView2DownloadOperation>,
    #[serde(skip)]
    pub last_measure: Option<(Instant, i64)>,
}

impl Download {
    pub fn refresh(&mut self) -> windows::core::Result<()> {
        let Some(op) = &self.operation else { return Ok(()) };
        // SAFETY: live COM interface, valid out-parameters, UI thread only.
        unsafe {
            op.BytesReceived(&mut self.received)?;
            op.TotalBytesToReceive(&mut self.total)?;
            let mut path = PWSTR::null();
            if op.ResultFilePath(&mut path).is_ok() {
                let p = take_pwstr(path);
                if !p.is_empty() {
                    self.path = p;
                }
            }
            let mut state = COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS;
            op.State(&mut state)?;
            let mut can_resume = BOOL(0);
            op.CanResume(&mut can_resume)?;
            self.resumable = can_resume.as_bool();
            self.state = if state == COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED { "completed" }
                else if state == COREWEBVIEW2_DOWNLOAD_STATE_INTERRUPTED { "interrupted" }
                else { "running" }.into();
            if state == COREWEBVIEW2_DOWNLOAD_STATE_INTERRUPTED {
                let mut reason = COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NONE;
                op.InterruptReason(&mut reason)?;
                self.reason = reason.0;
            }
            if state == COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED
                || (state == COREWEBVIEW2_DOWNLOAD_STATE_INTERRUPTED && !self.resumable) {
                self.operation = None;
            }
        }

        if self.state == "running" {
            let now = Instant::now();
            if let Some((prev_time, prev_bytes)) = self.last_measure {
                let elapsed = now.duration_since(prev_time).as_secs_f64();
                if elapsed >= 0.25 {
                    let diff_bytes = (self.received - prev_bytes).max(0);
                    let inst_speed = (diff_bytes as f64 / elapsed) as i64;
                    self.speed = if self.speed > 0 {
                        ((self.speed as f64 * 0.6) + (inst_speed as f64 * 0.4)) as i64
                    } else {
                        inst_speed
                    };
                    self.last_measure = Some((now, self.received));
                }
            } else {
                self.last_measure = Some((now, self.received));
                self.speed = 0;
            }
        } else {
            self.speed = 0;
            self.last_measure = None;
        }

        Ok(())
    }

    pub fn action(&mut self, action: &str) -> windows::core::Result<()> {
        if let Some(op) = &self.operation {
            // SAFETY: operations are owned and used exclusively by the STA UI thread.
            unsafe {
                match action {
                    "pause" => op.Pause()?,
                    "resume" if self.resumable => op.Resume()?,
                    "cancel" => {
                        op.Cancel()?;
                        self.operation = None;
                        self.state = "cancelled".into();
                        self.resumable = false;
                        self.speed = 0;
                        self.last_measure = None;
                        return Ok(());
                    }
                    _ => return Ok(()),
                }
            }
            self.refresh()?;
        }
        Ok(())
    }
}

pub fn delete_file(path: &str) {
    let path = path.trim();
    if path.is_empty() { return; }
    let mut p = std::path::PathBuf::from(path);
    if !p.exists() {
        if let Some(dl) = get_downloads_path() {
            let in_dl = dl.join(path);
            if in_dl.exists() {
                p = in_dl;
            }
        }
    }
    if !p.exists() { return; }

    use std::os::windows::ffi::OsStrExt;
    let mut wide: Vec<u16> = p.as_os_str().encode_wide().collect();
    wide.push(0);
    wide.push(0); // SHFileOperationW requires double null termination

    use windows::Win32::UI::Shell::{SHFileOperationW, SHFILEOPSTRUCTW, FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_SILENT, FOF_NOERRORUI};
    unsafe {
        let mut op = SHFILEOPSTRUCTW {
            hwnd: windows::Win32::Foundation::HWND(std::ptr::null_mut()),
            wFunc: FO_DELETE,
            pFrom: windows::core::PCWSTR(wide.as_ptr()),
            pTo: windows::core::PCWSTR::null(),
            fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI).0 as u16,
            fAnyOperationsAborted: windows::Win32::Foundation::BOOL(0),
            hNameMappings: std::ptr::null_mut(),
            lpszProgressTitle: windows::core::PCWSTR::null(),
        };
        let res = SHFileOperationW(&mut op);
        if res != 0 {
            let _ = std::fs::remove_file(&p);
        }
    }
}

pub fn open_file(path: &str) {
    use windows::core::HSTRING;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let path = path.trim();
    if path.is_empty() { return; }
    let mut p = std::path::PathBuf::from(path);
    if !p.exists() {
        if let Some(dl) = get_downloads_path() {
            let in_dl = dl.join(path);
            if in_dl.exists() {
                p = in_dl;
            }
        }
    }
    unsafe {
        let _ = ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(p.to_string_lossy().as_ref()),
            None,
            None,
            SW_SHOWNORMAL,
        );
    }
}

pub fn show_in_folder(path: &str) {
    use std::os::windows::process::CommandExt;
    let path = path.trim();
    if path.is_empty() {
        open_downloads_folder();
        return;
    }
    let mut p = std::path::PathBuf::from(path);
    if !p.exists() {
        if let Some(dl) = get_downloads_path() {
            let in_dl = dl.join(path);
            if in_dl.exists() {
                p = in_dl;
            }
        }
    }
    if p.exists() {
        let _ = std::process::Command::new("explorer.exe")
            .raw_arg(format!("/select,\"{}\"", p.to_string_lossy()))
            .spawn();
    } else if let Some(parent) = p.parent() {
        if parent.exists() && !parent.as_os_str().is_empty() {
            let _ = std::process::Command::new("explorer.exe")
                .arg(parent)
                .spawn();
        } else {
            open_downloads_folder();
        }
    } else {
        open_downloads_folder();
    }
}

pub fn get_downloads_path() -> Option<std::path::PathBuf> {
    use windows::Win32::UI::Shell::{SHGetKnownFolderPath, FOLDERID_Downloads, KNOWN_FOLDER_FLAG};
    unsafe {
        if let Ok(path_pwstr) = SHGetKnownFolderPath(&FOLDERID_Downloads, KNOWN_FOLDER_FLAG(0), None) {
            let path_str = take_pwstr(path_pwstr);
            if !path_str.is_empty() {
                return Some(std::path::PathBuf::from(path_str));
            }
        }
    }
    None
}

pub fn open_downloads_folder() {
    let _ = std::process::Command::new("explorer.exe")
        .arg("shell:Downloads")
        .spawn();
}

pub fn setup_window(window: &tao::window::Window) {
    use windows::Win32::{
        Foundation::{BOOL, COLORREF, HWND},
        Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE},
        Graphics::Gdi::CreateSolidBrush,
        UI::WindowsAndMessaging::{SetClassLongPtrW, SetWindowPos, GCLP_HBRBACKGROUND, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER},
    };
    use tao::platform::windows::WindowExtWindows;

    let hwnd = HWND(window.hwnd() as _);
    unsafe {
        let dark = BOOL(1);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const _ as _,
            std::mem::size_of::<BOOL>() as u32,
        );
        let border_color: u32 = 0x0016110E; // #0e1116
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &border_color as *const _ as _,
            std::mem::size_of::<u32>() as u32,
        );
        let dark_brush = CreateSolidBrush(COLORREF(0x0016110E));
        let _ = SetClassLongPtrW(hwnd, GCLP_HBRBACKGROUND, dark_brush.0 as _);
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
        );
    }
}

pub fn app_icon() -> Option<tao::window::Icon> {
    const SIZE: u32 = 32;
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    let tan_skew = 14.0f64.to_radians().tan();
    let scale = SIZE as f64 / 64.0;
    for y in 0..SIZE {
        let y_f = y as f64;
        for x in 0..SIZE {
            let x_f = x as f64;
            let mut alpha_acc = 0.0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let sub_x = (x_f + (sx as f64 + 0.5) / 4.0) / scale;
                    let sub_y = (y_f + (sy as f64 + 0.5) / 4.0) / scale;
                    let unskew_x = sub_x - (sub_y - 31.0) * tan_skew;
                    let in_vert = unskew_x >= 15.0 && unskew_x <= 29.0 && sub_y >= 5.0 && sub_y <= 57.0;
                    let in_top = unskew_x >= 15.0 && unskew_x <= 48.0 && sub_y >= 5.0 && sub_y <= 18.0;
                    let in_mid = unskew_x >= 15.0 && unskew_x <= 45.0 && sub_y >= 24.0 && sub_y <= 37.0;
                    if in_vert || in_top || in_mid {
                        alpha_acc += 1.0;
                    }
                }
            }
            let alpha = alpha_acc / 16.0;
            let idx = ((y * SIZE + x) * 4) as usize;
            if alpha > 0.0 {
                rgba[idx + 0] = 159; // R
                rgba[idx + 1] = 232; // G
                rgba[idx + 2] = 195; // B
                rgba[idx + 3] = (255.0 * alpha) as u8; // A
            }
        }
    }
    tao::window::Icon::from_rgba(rgba, SIZE, SIZE).ok()
}

pub fn core(view: &WebView) -> windows::core::Result<ICoreWebView2> {
    // SAFETY: wry owns a valid controller on this UI thread.
    unsafe { view.controller().CoreWebView2() }
}

pub fn shortcuts(view: &WebView, proxy: EventLoopProxy<UserEvent>) -> windows::core::Result<()> {
    let handler = AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        // SAFETY: callback arguments are valid for the synchronous invocation.
        unsafe {
            let mut kind = COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN;
            args.KeyEventKind(&mut kind)?;
            if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN { return Ok(()); }
            let mut key = 0;
            args.VirtualKey(&mut key)?;
            let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
            let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
            let action = match (ctrl, key) {
                (true, 0x4C) => "focus",
                (true, 0x54) => "new",
                (true, 0x57) => "close_active",
                (true, 0x4A) => "downloads",
                (true, 0x52) | (false, 0x74) => "reload",
                (true, 0x09) if shift => "previous_tab",
                (true, 0x09) => "next_tab",
                _ => return Ok(()),
            };
            args.SetHandled(true)?;
            // Defer mutations: WebView2 forbids reentrant controller calls here.
            let _ = proxy.send_event(UserEvent::Command(None, Command::simple(action)));
        }
        Ok(())
    }));
    let mut token = Default::default();
    // SAFETY: COM retains the handler until controller destruction.
    unsafe { view.controller().add_AcceleratorKeyPressed(&handler, &mut token) }
}

pub fn attach(
    view: &WebView,
    tab: u64,
    proxy: EventLoopProxy<UserEvent>,
    block_enabled: Rc<Cell<bool>>,
    blocked_count: Rc<Cell<u64>>,
    downloads: Downloads,
    next_download: Rc<Cell<u64>>,
) -> windows::core::Result<()> {
    let core = core(view)?;
    shortcuts(view, proxy.clone())?;
    // SAFETY: all interfaces, handlers and out-parameters live on this UI thread.
    unsafe {
        let settings = core.Settings()?;
        settings.SetIsWebMessageEnabled(false)?;
        settings.SetAreHostObjectsAllowed(false)?;
        let environment = core.cast::<ICoreWebView2_2>()?.Environment()?;
        // Require a recent Evergreen Runtime to cover document AND worker requests.
        core.cast::<ICoreWebView2_22>()?.AddWebResourceRequestedFilterWithRequestSourceKinds(
            w!("*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
            COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
        )?;
        let block = WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
            if !block_enabled.get() { return Ok(()); }
            if let Some(args) = args {
                let request = args.Request()?;
                let mut uri = PWSTR::null();
                request.Uri(&mut uri)?;
                let uri = take_pwstr(uri);
                if policy::blocked(&uri) {
                    let response = environment.CreateWebResourceResponse(
                        None::<&windows::Win32::System::Com::IStream>, 403,
                        w!("Blocked by Fastium"), w!("Content-Length: 0\r\nCache-Control: no-store\r\n"),
                    )?;
                    args.SetResponse(&response)?;
                    blocked_count.set(blocked_count.get().saturating_add(1));
                }
            }
            Ok(())
        }));
        let mut token = Default::default();
        core.add_WebResourceRequested(&block, &mut token)?;

        let source_proxy = proxy.clone();
        let source = SourceChangedEventHandler::create(Box::new(move |sender, _| {
            if let Some(sender) = sender {
                let mut uri = PWSTR::null();
                sender.Source(&mut uri)?;
                let _ = source_proxy.send_event(UserEvent::Source(tab, take_pwstr(uri)));
            }
            Ok(())
        }));
        core.add_SourceChanged(&source, &mut token)?;

        let history_proxy = proxy.clone();
        let history = HistoryChangedEventHandler::create(Box::new(move |sender, _| {
            if let Some(sender) = sender {
                let mut back = BOOL(0);
                let mut forward = BOOL(0);
                sender.CanGoBack(&mut back)?;
                sender.CanGoForward(&mut forward)?;
                let _ = history_proxy.send_event(UserEvent::History(tab, back.as_bool(), forward.as_bool()));
            }
            Ok(())
        }));
        core.add_HistoryChanged(&history, &mut token)?;

        if let Ok(core15) = core.cast::<ICoreWebView2_15>() {
            let favicon_proxy = proxy.clone();
            let favicon = FaviconChangedEventHandler::create(Box::new(move |sender, _| {
                if let Some(sender) = sender {
                    if let Ok(c15) = sender.cast::<ICoreWebView2_15>() {
                        let mut uri = PWSTR::null();
                        if c15.FaviconUri(&mut uri).is_ok() {
                            let uri_str = take_pwstr(uri);
                            if !uri_str.is_empty() {
                                let _ = favicon_proxy.send_event(UserEvent::Favicon(tab, uri_str));
                            }
                        }
                    }
                }
                Ok(())
            }));
            core15.add_FaviconChanged(&favicon, &mut token)?;
        }

        let download = DownloadStartingEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut list = downloads.borrow_mut();
            // Bounded history, but never evict a live operation.
            while list.len() >= 100 {
                if let Some(i) = list.iter().position(|d| d.operation.is_none()) { list.remove(i); }
                else { args.SetCancel(true)?; return Ok(()); }
            }
            let operation = args.DownloadOperation()?;
            let mut initial_path = String::new();
            let mut path_pwstr = PWSTR::null();
            if args.ResultFilePath(&mut path_pwstr).is_ok() {
                initial_path = take_pwstr(path_pwstr);
            }
            if initial_path.is_empty() {
                if operation.ResultFilePath(&mut path_pwstr).is_ok() {
                    initial_path = take_pwstr(path_pwstr);
                }
            }
            let id = next_download.get();
            next_download.set(id + 1);
            list.push(Download { id, tab, path: initial_path, received: 0, total: 0,
                speed: 0, state: "running".into(), resumable: false, reason: 0, operation: Some(operation),
                last_measure: Some((Instant::now(), 0)) });
            args.SetHandled(true)?;
            let _ = proxy.send_event(UserEvent::DownloadStarted);
            Ok(())
        }));
        core.cast::<ICoreWebView2_4>()?.add_DownloadStarting(&download, &mut token)?;
    }
    Ok(())
}

pub fn playing_audio(view: &WebView) -> bool {
    let result = (|| -> windows::core::Result<bool> {
        let core = core(view)?.cast::<ICoreWebView2_8>()?;
        let mut playing = BOOL(0);
        // SAFETY: valid interface and stack out-parameter on UI thread.
        unsafe { core.IsDocumentPlayingAudio(&mut playing)?; }
        Ok(playing.as_bool())
    })();
    result.unwrap_or(true) // If detection fails, leave the tab running.
}

pub fn suspend(view: &WebView, id: u64, proxy: EventLoopProxy<UserEvent>) -> windows::core::Result<()> {
    let core = core(view)?.cast::<ICoreWebView2_3>()?;
    let callback = TrySuspendCompletedHandler::create(Box::new(move |result, success| {
        let _ = proxy.send_event(UserEvent::Suspended(id, result.is_ok() && success));
        Ok(())
    }));
    // SAFETY: callback is retained by COM until asynchronous completion.
    unsafe { core.TrySuspend(&callback) }
}

pub fn resume(view: &WebView) -> windows::core::Result<()> {
    // SAFETY: live interface on its owning UI thread.
    unsafe { core(view)?.cast::<ICoreWebView2_3>()?.Resume() }
}

pub fn reparent_view(view: &WebView, new_window: &tao::window::Window) -> windows::core::Result<()> {
    use tao::platform::windows::WindowExtWindows;
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::SetParent,
    };
    unsafe {
        let new_parent = HWND(new_window.hwnd() as _);
        let mut container_hwnd = HWND::default();
        if view.controller().ParentWindow(&mut container_hwnd).is_ok() && !container_hwnd.0.is_null() {
            let _ = SetParent(container_hwnd, new_parent);
        } else {
            view.controller().SetParentWindow(new_parent)?;
        }
    }
    Ok(())
}

#[allow(dead_code)]
pub fn current_uri(view: &WebView) -> Option<String> {
    unsafe {
        let mut uri = PWSTR::null();
        if core(view).ok()?.Source(&mut uri).is_ok() {
            let s = take_pwstr(uri);
            if !s.is_empty() { return Some(s); }
        }
    }
    None
}
