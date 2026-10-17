// Turns Don't Forget off on chosen websites. A background thread watches the foreground
// browser's address bar through UI Automation, only re-reading it when the window or its
// title changes, and publishes that window as BLOCKED_WINDOW for the mouse hook.
use crate::hook::process_name;
use std::{
    sync::{atomic::*, Mutex},
    thread,
    time::{Duration, Instant},
};
use windows::{
    core::VARIANT,
    Win32::{
        Foundation::HWND as WHWND,
        System::Com::*,
        UI::Accessibility::*,
    },
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub static SITES: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// The browser window currently showing an ignored site (0 if none).
pub static BLOCKED_WINDOW: AtomicIsize = AtomicIsize::new(0);

const BROWSERS: [&str; 11] = [
    "chrome", "msedge", "brave", "firefox", "opera", "vivaldi", "arc", "zen", "librewolf", "waterfox", "floorp",
];

/// "https://www.YouTube.com/watch" -> "youtube.com/watch"
pub fn normalize(s: &str) -> String {
    let s = s.trim().to_lowercase();
    let s = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")).unwrap_or(&s);
    let s = s.strip_prefix("www.").unwrap_or(s);
    s.trim_end_matches('/').to_string()
}

/// A site matches its own host, its subdomains, and (if it has a path) anything under that path.
pub fn matches(url: &str, sites: &[String]) -> bool {
    let url = normalize(url);
    let host = url.split(['/', '?', '#']).next().unwrap_or("");
    sites.iter().any(|site| {
        if site.contains('/') {
            url.starts_with(site.as_str())
        } else {
            host == site || host.ends_with(&format!(".{site}"))
        }
    })
}

fn is_browser(hwnd: isize) -> bool {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd as _, &mut pid) };
    let name = process_name(pid);
    BROWSERS.contains(&name.as_str())
}

fn title(hwnd: isize) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(hwnd as _, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// Reads the address bar of a browser window: the first editable text field with a value.
fn read_url(uia: &IUIAutomation, hwnd: isize) -> Option<String> {
    unsafe {
        let root = uia.ElementFromHandle(WHWND(hwnd as _)).ok()?;
        let cond = uia
            .CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_EditControlTypeId.0))
            .ok()?;
        let edit = root.FindFirst(TreeScope_Descendants, &cond).ok()?;
        let pattern: IUIAutomationValuePattern = edit.GetCurrentPatternAs(UIA_ValuePatternId).ok()?;
        let value = pattern.CurrentValue().ok()?.to_string();
        (!value.is_empty()).then_some(value)
    }
}

fn automation() -> Option<IUIAutomation> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()
    }
}

pub fn start() {
    thread::spawn(|| {
        let mut uia: Option<IUIAutomation> = None;
        let mut last = (0isize, String::new());
        let mut checked = Instant::now();
        loop {
            thread::sleep(Duration::from_millis(300));
            let sites = SITES.lock().unwrap().clone();
            if sites.is_empty() {
                BLOCKED_WINDOW.store(0, Ordering::Relaxed);
                continue;
            }
            let fg = unsafe { GetForegroundWindow() } as isize;
            let key = (fg, title(fg));
            // re-read on window/title change, and every few seconds for pages that keep their title
            if key == last && checked.elapsed() < Duration::from_secs(3) {
                continue;
            }
            last = key;
            checked = Instant::now();
            if !is_browser(fg) {
                continue; // keep the last browser's state; the hook compares window handles
            }
            if uia.is_none() {
                uia = automation();
            }
            let blocked = uia.as_ref().and_then(|u| read_url(u, fg)).is_some_and(|url| matches(&url, &sites));
            BLOCKED_WINDOW.store(if blocked { fg } else { 0 }, Ordering::Relaxed);
        }
    });
}

/// `DontForget.exe --probe-url`: prints the address of every open browser window and exits.
pub fn probe() {
    let Some(uia) = automation() else {
        println!("UI Automation unavailable");
        return;
    };
    let mut wins: Vec<isize> = Vec::new();
    unsafe extern "system" fn collect(h: windows_sys::Win32::Foundation::HWND, l: isize) -> i32 {
        let v = &mut *(l as *mut Vec<isize>);
        if IsWindowVisible(h) != 0 && GetWindowTextLengthW(h) > 0 {
            v.push(h as isize);
        }
        1
    }
    unsafe { EnumWindows(Some(collect), &mut wins as *mut _ as isize) };
    for h in wins.into_iter().filter(|&h| is_browser(h)) {
        let t = Instant::now();
        let url = read_url(&uia, h).unwrap_or_else(|| "<no address bar found>".into());
        println!("{} | {} | {} ms", title(h), url, t.elapsed().as_millis());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_matching() {
        let sites = vec![normalize("YouTube.com"), normalize("https://github.com/settings/")];
        assert!(matches("youtube.com/watch?v=1", &sites));
        assert!(matches("https://www.youtube.com", &sites));
        assert!(matches("m.youtube.com/feed", &sites));
        assert!(!matches("notyoutube.com", &sites));
        assert!(!matches("youtube.com.evil.net/x", &sites));
        assert!(matches("github.com/settings/profile", &sites));
        assert!(!matches("github.com/BeForce1", &sites));
    }
}
