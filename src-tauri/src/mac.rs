//! macOS: Option+right-click in Finder, and the permissions Convertino needs.
//!
//! An event tap (it needs Accessibility permission) watches right-button
//! presses. With Option held over a Finder window or the desktop, the press
//! and release are swallowed so Finder's menu never opens, and the callback
//! runs with the release point. The file under the pointer is found through
//! the Accessibility API (Finder exposes each item's URL); if that fails, the
//! Finder selection is used, as with the hotkey.
//!
//! Exposes the same names as the Windows module (alt_click.rs): ENABLED,
//! RING_ACTIVE, install, on_click_while_ring.
//!
//! Plain C APIs only (CoreGraphics, CoreFoundation, HIServices, Apple Events).

#![allow(non_upper_case_globals, non_snake_case, clippy::missing_safety_doc)]

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFArrayRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CFIndex = isize;
type CFMachPortRef = *mut c_void;
type CFRunLoopSourceRef = *mut c_void;
type CFRunLoopRef = *mut c_void;
type CGEventRef = *mut c_void;
type CGEventTapProxy = *mut c_void;
type AXUIElementRef = *const c_void;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct CGRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

type TapCallback = extern "C" fn(CGEventTapProxy, u32, CGEventRef, *mut c_void) -> CGEventRef;

/// A C struct only ever used by address.
#[repr(C)]
struct Opaque {
    _private: [u8; 0],
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopCommonModes: CFStringRef;
    static kCFBooleanTrue: CFTypeRef;
    static kCFTypeDictionaryKeyCallBacks: Opaque;
    static kCFTypeDictionaryValueCallBacks: Opaque;
    fn CFMachPortCreateRunLoopSource(alloc: CFTypeRef, port: CFMachPortRef, order: CFIndex) -> CFRunLoopSourceRef;
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRun();
    fn CFRelease(cf: CFTypeRef);
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFURLGetTypeID() -> usize;
    fn CFStringCreateWithCString(alloc: CFTypeRef, s: *const c_char, encoding: u32) -> CFStringRef;
    fn CFStringGetCString(s: CFStringRef, buf: *mut c_char, size: CFIndex, encoding: u32) -> bool;
    fn CFArrayGetCount(a: CFArrayRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(a: CFArrayRef, i: CFIndex) -> CFTypeRef;
    fn CFDictionaryGetValue(d: CFDictionaryRef, key: CFTypeRef) -> CFTypeRef;
    fn CFDictionaryCreate(
        alloc: CFTypeRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        n: CFIndex,
        key_cb: *const c_void,
        value_cb: *const c_void,
    ) -> CFDictionaryRef;
    fn CFNumberGetValue(n: CFTypeRef, kind: CFIndex, out: *mut c_void) -> bool;
    fn CFURLGetFileSystemRepresentation(url: CFTypeRef, resolve: bool, buf: *mut u8, len: CFIndex) -> bool;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(tap: u32, place: u32, options: u32, mask: u64, callback: TapCallback, user: *mut c_void) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetLocation(event: CGEventRef) -> CGPoint;
    fn CGEventGetFlags(event: CGEventRef) -> u64;
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> CFArrayRef;
    fn CGRectMakeWithDictionaryRepresentation(dict: CFDictionaryRef, rect: *mut CGRect) -> bool;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    static kAXTrustedCheckOptionPrompt: CFStringRef;
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCopyElementAtPosition(app: AXUIElementRef, x: f32, y: f32, element: *mut AXUIElementRef) -> i32;
    fn AXUIElementCopyAttributeValue(element: AXUIElementRef, attribute: CFStringRef, value: *mut CFTypeRef) -> i32;
}

#[repr(C)]
struct AEDesc {
    descriptor_type: u32,
    data_handle: *mut c_void,
}

#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    fn AECreateDesc(kind: u32, data: *const c_void, size: isize, result: *mut AEDesc) -> i16;
    fn AEDisposeDesc(desc: *mut AEDesc) -> i16;
    fn AEDeterminePermissionToAutomateTarget(target: *const AEDesc, class: u32, id: u32, ask: bool) -> i32;
}

const kCFStringEncodingUTF8: u32 = 0x0800_0100;
const kCFNumberSInt64Type: CFIndex = 4;

const kCGSessionEventTap: u32 = 1;
const kCGHeadInsertEventTap: u32 = 0;
const kCGEventTapOptionDefault: u32 = 0;
const LEFT_DOWN: u32 = 1;
const RIGHT_DOWN: u32 = 3;
const RIGHT_UP: u32 = 4;
const TAP_DISABLED_TIMEOUT: u32 = 0xFFFF_FFFE;
const TAP_DISABLED_USER: u32 = 0xFFFF_FFFF;
const FLAG_ALTERNATE: u64 = 0x0008_0000;
const FLAG_COMMAND: u64 = 0x0010_0000;
const FLAG_CONTROL: u64 = 0x0004_0000;
const kCGWindowListOptionOnScreenOnly: u32 = 1;

fn fourcc(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

/// A CFString that is released when dropped.
struct CfStr(CFStringRef);
impl CfStr {
    fn new(s: &str) -> CfStr {
        let c = CString::new(s).unwrap_or_default();
        CfStr(unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), kCFStringEncodingUTF8) })
    }
}
impl Drop for CfStr {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) }
        }
    }
}

unsafe fn cf_string(s: CFTypeRef) -> Option<String> {
    if s.is_null() || CFGetTypeID(s) != CFStringGetTypeID() {
        return None;
    }
    let mut buf = vec![0 as c_char; 1024];
    if CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as CFIndex, kCFStringEncodingUTF8) {
        Some(CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
    } else {
        None
    }
}

unsafe fn cf_i64(n: CFTypeRef) -> Option<i64> {
    if n.is_null() || CFGetTypeID(n) != CFNumberGetTypeID() {
        return None;
    }
    let mut v: i64 = 0;
    CFNumberGetValue(n, kCFNumberSInt64Type, &mut v as *mut i64 as *mut c_void).then_some(v)
}

unsafe fn cf_url_path(u: CFTypeRef) -> Option<PathBuf> {
    if u.is_null() || CFGetTypeID(u) != CFURLGetTypeID() {
        return None;
    }
    let mut buf = vec![0u8; 4096];
    if CFURLGetFileSystemRepresentation(u, true, buf.as_mut_ptr(), buf.len() as CFIndex) {
        let s = CStr::from_ptr(buf.as_ptr() as *const c_char).to_string_lossy().into_owned();
        Some(PathBuf::from(s))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Option+right-click

/// Called with the release point (global screen points, top-left origin).
type Callback = Box<dyn Fn(f64, f64) + Send + Sync>;

static CALLBACK: OnceLock<Callback> = OnceLock::new();
static AWAY: OnceLock<Box<dyn Fn(f64, f64) + Send + Sync>> = OnceLock::new();
/// Option+right-click opens the wheel (Settings can turn it off).
pub static ENABLED: AtomicBool = AtomicBool::new(true);
/// True while the progress ring is on screen.
pub static RING_ACTIVE: AtomicBool = AtomicBool::new(false);
static ARMED: AtomicBool = AtomicBool::new(false);
static TAP: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
/// The tap is running (Accessibility was granted).
static RUNNING: AtomicBool = AtomicBool::new(false);

/// What to do with clicks elsewhere while the ring is showing.
pub fn on_click_while_ring(callback: impl Fn(f64, f64) + Send + Sync + 'static) {
    let _ = AWAY.set(Box::new(callback));
}

/// Option+right-click works right now (the tap is installed).
pub fn alt_click_running() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

/// Starts the event tap on its own thread. Without Accessibility permission
/// it can't start; it tries again every two seconds, so it works as soon as
/// the permission is given, without restarting Convertino.
pub fn install(callback: impl Fn(f64, f64) + Send + Sync + 'static) {
    if CALLBACK.set(Box::new(callback)).is_err() {
        return;
    }
    std::thread::spawn(|| unsafe {
        let mask = (1u64 << LEFT_DOWN) | (1u64 << RIGHT_DOWN) | (1u64 << RIGHT_UP);
        let mut told = false;
        let tap = loop {
            let tap = CGEventTapCreate(kCGSessionEventTap, kCGHeadInsertEventTap, kCGEventTapOptionDefault, mask, tap_callback, std::ptr::null_mut());
            if !tap.is_null() {
                break tap;
            }
            if !told {
                log::info!("Option+right-click waits for Accessibility permission");
                told = true;
            }
            std::thread::sleep(Duration::from_secs(2));
        };
        TAP.store(tap, Ordering::SeqCst);
        let source = CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0);
        CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
        RUNNING.store(true, Ordering::SeqCst);
        log::info!("Option+right-click is on");
        CFRunLoopRun();
    });
}

extern "C" fn tap_callback(_proxy: CGEventTapProxy, kind: u32, event: CGEventRef, _user: *mut c_void) -> CGEventRef {
    unsafe {
        match kind {
            // macOS switches a slow tap off; switch it back on.
            TAP_DISABLED_TIMEOUT | TAP_DISABLED_USER => {
                let tap = TAP.load(Ordering::SeqCst);
                if !tap.is_null() {
                    CGEventTapEnable(tap, true);
                }
                event
            }
            LEFT_DOWN => {
                if RING_ACTIVE.load(Ordering::SeqCst) {
                    let p = CGEventGetLocation(event);
                    if let Some(away) = AWAY.get() {
                        away(p.x, p.y);
                    }
                }
                event
            }
            RIGHT_DOWN => {
                let flags = CGEventGetFlags(event);
                let option_only = flags & FLAG_ALTERNATE != 0 && flags & (FLAG_COMMAND | FLAG_CONTROL) == 0;
                if ENABLED.load(Ordering::SeqCst) && option_only && finder_at(CGEventGetLocation(event)) {
                    ARMED.store(true, Ordering::SeqCst);
                    std::ptr::null_mut() // swallowed: no context menu
                } else {
                    event
                }
            }
            RIGHT_UP if ARMED.swap(false, Ordering::SeqCst) => {
                let p = CGEventGetLocation(event);
                // The real work happens off the tap's thread, which must answer quickly.
                std::thread::spawn(move || {
                    if let Some(cb) = CALLBACK.get() {
                        cb(p.x, p.y);
                    }
                });
                std::ptr::null_mut()
            }
            _ => event,
        }
    }
}

/// The top window under the point belongs to Finder (a Finder window or the desktop).
fn finder_at(p: CGPoint) -> bool {
    unsafe {
        let list = CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly, 0);
        if list.is_null() {
            return false;
        }
        let (k_layer, k_bounds, k_owner) = (CfStr::new("kCGWindowLayer"), CfStr::new("kCGWindowBounds"), CfStr::new("kCGWindowOwnerName"));
        let mut result = false;
        for i in 0..CFArrayGetCount(list) {
            let w = CFArrayGetValueAtIndex(list, i);
            if w.is_null() || CFGetTypeID(w) != CFDictionaryGetTypeID() {
                continue;
            }
            // Menus, the Dock, the menu bar and panels sit above layer 0.
            let layer = cf_i64(CFDictionaryGetValue(w, k_layer.0)).unwrap_or(0);
            if layer > 0 {
                continue;
            }
            let b = CFDictionaryGetValue(w, k_bounds.0);
            let mut r = CGRect::default();
            if b.is_null() || !CGRectMakeWithDictionaryRepresentation(b, &mut r) {
                continue;
            }
            if p.x < r.x || p.y < r.y || p.x >= r.x + r.w || p.y >= r.y + r.h {
                continue;
            }
            result = cf_string(CFDictionaryGetValue(w, k_owner.0)).as_deref() == Some("Finder");
            break;
        }
        CFRelease(list);
        result
    }
}

/// The file or folder under the point in Finder, through the Accessibility API.
pub fn item_at(x: f64, y: f64) -> Option<PathBuf> {
    unsafe {
        let system = AXUIElementCreateSystemWide();
        if system.is_null() {
            return None;
        }
        let mut el: AXUIElementRef = std::ptr::null();
        let err = AXUIElementCopyElementAtPosition(system, x as f32, y as f32, &mut el);
        CFRelease(system);
        if err != 0 || el.is_null() {
            return None;
        }
        let (k_url, k_parent) = (CfStr::new("AXURL"), CfStr::new("AXParent"));
        // The element under the pointer is often the item's name or icon; its URL is on it or a parent.
        let mut current = el;
        let mut found = None;
        for _ in 0..3 {
            let mut value: CFTypeRef = std::ptr::null();
            if AXUIElementCopyAttributeValue(current, k_url.0, &mut value) == 0 && !value.is_null() {
                found = cf_url_path(value);
                CFRelease(value);
                if found.is_some() {
                    break;
                }
            }
            let mut parent: CFTypeRef = std::ptr::null();
            if AXUIElementCopyAttributeValue(current, k_parent.0, &mut parent) != 0 || parent.is_null() {
                break;
            }
            CFRelease(current);
            current = parent;
        }
        CFRelease(current);
        // A window's own URL is its folder: that's not "the item under the pointer".
        found.filter(|p| p.exists())
    }
}

// ---------------------------------------------------------------------------
// Permissions

/// Accessibility (System Settings > Privacy & Security > Accessibility): Option+right-click.
pub fn accessibility_granted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Shows macOS's own "Convertino would like to control this computer" prompt
/// (it adds Convertino to the list), then opens the Accessibility settings.
pub fn ask_accessibility() {
    unsafe {
        let keys = [kAXTrustedCheckOptionPrompt as CFTypeRef];
        let values = [kCFBooleanTrue];
        let dict = CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            &kCFTypeDictionaryKeyCallBacks as *const Opaque as *const c_void,
            &kCFTypeDictionaryValueCallBacks as *const Opaque as *const c_void,
        );
        let _ = AXIsProcessTrustedWithOptions(dict);
        if !dict.is_null() {
            CFRelease(dict);
        }
    }
    open_settings("Privacy_Accessibility");
}

/// Permission to ask Finder for its selection (Automation). "granted",
/// "denied", "not-asked", or "unknown" (e.g. Finder isn't running).
/// With `ask`, macOS shows its prompt if it hasn't yet (this blocks until answered).
pub fn finder_automation(ask: bool) -> &'static str {
    let id = b"com.apple.finder";
    unsafe {
        let mut desc = AEDesc { descriptor_type: 0, data_handle: std::ptr::null_mut() };
        if AECreateDesc(fourcc(b"bund"), id.as_ptr() as *const c_void, id.len() as isize, &mut desc) != 0 {
            return "unknown";
        }
        let r = AEDeterminePermissionToAutomateTarget(&desc, fourcc(b"****"), fourcc(b"****"), ask);
        AEDisposeDesc(&mut desc);
        match r {
            0 => "granted",
            -1743 => "denied",
            -1744 => "not-asked",
            _ => "unknown",
        }
    }
}

/// Opens a Privacy & Security pane of System Settings.
pub fn open_settings(anchor: &str) {
    let url = format!("x-apple.systempreferences:com.apple.preference.security?{anchor}");
    let _ = std::process::Command::new("/usr/bin/open").arg(url).spawn();
}

/// Serialises Finder prompts: one question at a time.
pub fn ask_finder_once() -> &'static str {
    static ONE: Mutex<()> = Mutex::new(());
    let _g = ONE.lock().unwrap_or_else(|e| e.into_inner());
    finder_automation(true)
}
