//! Alt+right-click on a file in Explorer or on the desktop opens the wheel.
//!
//! A low-level mouse hook watches right-button presses. When Alt is held and
//! the pointer is over Explorer's file view (or the desktop icons), the press
//! and release are swallowed so the normal context menu never appears, and
//! the callback runs with the release point. Everything else passes through.
//!
//! The hook must answer within a few hundred milliseconds or Windows removes
//! it, so it only checks window classes; the real work happens on a thread.
//!
//! While the progress ring is showing, the same hook also reports any other
//! click (the ring lets clicks through), so a running job can move to the
//! corner card as soon as you carry on working.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::OnceLock;

use ::windows::core::PCWSTR;
use ::windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use ::windows::Win32::System::LibraryLoader::GetModuleHandleW;
use ::windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_MENU,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetAncestor, GetClassNameW, GetMessageW, SetWindowsHookExW,
    TranslateMessage, WindowFromPoint, GA_PARENT, GA_ROOT, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT,
    WH_MOUSE_LL, WM_LBUTTONDOWN, WM_RBUTTONDOWN, WM_RBUTTONUP,
};

/// Called with the release point (physical screen px) and the top-level window.
type Callback = Box<dyn Fn(i32, i32, isize) + Send + Sync>;

static CALLBACK: OnceLock<Callback> = OnceLock::new();
/// Called with the click point (physical screen px) while the ring is active.
static AWAY: OnceLock<Box<dyn Fn(i32, i32) + Send + Sync>> = OnceLock::new();
/// Alt+right-click opens the wheel (Settings can turn it off; the hook stays
/// installed for the progress ring's clicks).
pub static ENABLED: AtomicBool = AtomicBool::new(true);
/// True while the progress ring is on screen.
pub static RING_ACTIVE: AtomicBool = AtomicBool::new(false);
static ARMED: AtomicBool = AtomicBool::new(false);
static ARMED_ROOT: AtomicIsize = AtomicIsize::new(0);

/// What to do with clicks elsewhere while the ring is showing.
pub fn on_click_while_ring(callback: impl Fn(i32, i32) + Send + Sync + 'static) {
    let _ = AWAY.set(Box::new(callback));
}

/// Starts the hook on its own thread (it needs a message loop).
pub fn install(callback: impl Fn(i32, i32, isize) + Send + Sync + 'static) {
    if CALLBACK.set(Box::new(callback)).is_err() {
        return; // already installed
    }
    std::thread::spawn(|| unsafe {
        let module = GetModuleHandleW(PCWSTR::null()).ok().map(HINSTANCE::from);
        match SetWindowsHookExW(WH_MOUSE_LL, Some(hook), module, 0) {
            Ok(_) => log::info!("Alt+right-click is on"),
            Err(e) => {
                log::error!("couldn't turn on Alt+right-click: {}", e.message());
                return;
            }
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}

unsafe fn class_of(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    let len = GetClassNameW(hwnd, &mut buf);
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

/// The top-level window if `pt` is over a file view we handle.
unsafe fn file_view_root(pt: POINT) -> Option<HWND> {
    let under = WindowFromPoint(pt);
    if under.is_invalid() {
        return None;
    }
    let root = GetAncestor(under, GA_ROOT);
    let root_class = class_of(root);
    let under_class = class_of(under);
    let parent_class = class_of(GetAncestor(under, GA_PARENT));
    let is_item_view = match root_class.as_str() {
        // Explorer: the item list is a DirectUIHWND inside SHELLDLL_DefView.
        "CabinetWClass" | "ExploreWClass" => under_class == "DirectUIHWND" && parent_class == "SHELLDLL_DefView",
        // Desktop icons.
        "Progman" | "WorkerW" => under_class == "SysListView32",
        // Open and Save dialogs host the same file view.
        "#32770" => under_class == "DirectUIHWND" && parent_class == "SHELLDLL_DefView",
        _ => false,
    };
    is_item_view.then_some(root)
}

/// Stops Explorer from showing its keyboard shortcuts when Alt is released:
/// a key press in between (an unassigned virtual key) cancels the "Alt alone" gesture.
unsafe fn mask_alt() {
    let key = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0xE8),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
    SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
}

unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
        let injected = info.flags & LLMHF_INJECTED != 0;
        let msg = wparam.0 as u32;

        let alt_held = (GetAsyncKeyState(VK_MENU.0 as i32) as u16 & 0x8000) != 0;
        // (An Alt+right-click opens a new wheel, which moves the ring's job on by itself.)
        let opens_wheel = msg == WM_RBUTTONDOWN && alt_held;
        if !injected && !opens_wheel && (msg == WM_LBUTTONDOWN || msg == WM_RBUTTONDOWN) && RING_ACTIVE.load(Ordering::SeqCst) {
            let (x, y) = (info.pt.x, info.pt.y);
            std::thread::spawn(move || {
                if let Some(cb) = AWAY.get() {
                    cb(x, y);
                }
            });
            // Never swallowed: the click still reaches whatever is under the pointer.
        }

        if !injected && msg == WM_RBUTTONDOWN && ENABLED.load(Ordering::SeqCst) {
            let alt_down = (GetAsyncKeyState(VK_MENU.0 as i32) as u16 & 0x8000) != 0;
            if alt_down {
                if let Some(root) = file_view_root(info.pt) {
                    ARMED.store(true, Ordering::SeqCst);
                    ARMED_ROOT.store(root.0 as isize, Ordering::SeqCst);
                    mask_alt();
                    return LRESULT(1); // swallow: no context menu
                }
            }
        } else if !injected && msg == WM_RBUTTONUP && ARMED.swap(false, Ordering::SeqCst) {
            let (x, y) = (info.pt.x, info.pt.y);
            let root = ARMED_ROOT.load(Ordering::SeqCst);
            std::thread::spawn(move || {
                if let Some(cb) = CALLBACK.get() {
                    cb(x, y, root);
                }
            });
            return LRESULT(1);
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}
