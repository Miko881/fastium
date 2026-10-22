#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod policy;
#[cfg(windows)]
mod browser;
#[cfg(windows)]
mod native;

#[cfg(windows)]
fn main() {
    if let Err(error) = browser::run() {
        use windows::{core::HSTRING, Win32::UI::WindowsAndMessaging::*};
        let message = HSTRING::from(format!(
            "Nie można uruchomić Fastium.\n\n{error}\n\nSprawdź instalację Microsoft Edge WebView2 Evergreen Runtime."
        ));
        // SAFETY: strings outlive the synchronous dialog; no parent window is required.
        unsafe { MessageBoxW(None, &message, windows::core::w!("Fastium"), MB_OK | MB_ICONERROR); }
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Fastium 0.1 wymaga Windows i WebView2. Polityki URL można testować na każdej platformie.");
}

