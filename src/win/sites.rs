// Turns Don't Forget off on chosen websites. A background thread watches the foreground
// browser's address bar through UI Automation, only re-reading it when the window or its
// title changes, and publishes that window as BLOCKED_WINDOW for the mouse hook.
use super::hook::process_name;
use crate::common::sites::{matches, BROWSERS};
use std::{
    sync::{atomic::*, Mutex, OnceLock},
    thread::{self, Thread},
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
static WATCHER: OnceLock<Thread> = OnceLock::new();



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

/// Call after changing SITES: the watcher sleeps for good while the list is empty.
pub fn wake() {
    if let Some(t) = WATCHER.get() {
        t.unpark();
    }
}

pub fn start() {
    let handle = thread::spawn(|| {
        let mut uia: Option<IUIAutomation> = None;
        let mut last = (0isize, String::new());
        let mut checked = Instant::now();
        loop {
            let sites = SITES.lock().unwrap().clone();
            if sites.is_empty() {
                BLOCKED_WINDOW.store(0, Ordering::Relaxed);
                thread::park();
                continue;
            }
            thread::sleep(Duration::from_millis(300));
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
    let _ = WATCHER.set(handle.thread().clone());
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

