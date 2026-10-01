//! Windows adapter: reads the selection from File Explorer or the desktop
//! through the Shell's COM interfaces.
//!
//! Flow: foreground window → matching entry in `ShellWindows` (for Win11
//! tabs, the entry whose browser window is the active tab) → its shell view
//! → `IFolderView2::GetSelection` → file-system paths.

use super::Selection;
use ::windows::core::{w, Interface, PCWSTR, PWSTR};
use ::windows::Win32::Foundation::HWND;
use ::windows::Win32::Foundation::POINT;
use ::windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IServiceProvider,
    CLSCTX_ALL, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED,
};
use ::windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationSelectionItemPattern, UIA_ListItemControlTypeId,
    UIA_SelectionItemPatternId,
};
use ::windows::Win32::System::Variant::{VARIANT, VT_I4};
use ::windows::Win32::UI::Shell::{
    IFolderView2, IShellBrowser, IShellWindows, IWebBrowser2, ShellWindows, CSIDL_DESKTOP,
    SID_STopLevelBrowser, SIGDN_FILESYSPATH, SWC_DESKTOP, SWFO_NEEDDISPATCH,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetClassNameW, GetForegroundWindow,
};

type Res<T> = Result<T, String>;

fn err(context: &str) -> impl Fn(::windows::core::Error) -> String + '_ {
    move |e| format!("{context}: {}", e.message())
}

/// Balances CoInitializeEx on this thread.
struct ComGuard(bool);
impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

/// The selection in the active Explorer window or on the desktop.
pub fn current_selection() -> Res<Selection> {
    selection_for(None)
}

/// The selection in a given top-level window (from Alt+right-click), or in
/// the foreground window when `root` is None.
pub fn selection_for(root: Option<isize>) -> Res<Selection> {
    unsafe {
        let _com = ComGuard(CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok());

        let fg = match root {
            Some(h) => HWND(h as *mut core::ffi::c_void),
            None => GetForegroundWindow(),
        };
        if fg.is_invalid() {
            return Err("No window is active".into());
        }
        let class = class_name(fg);
        let shell_windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)
            .map_err(err("Couldn't reach the Windows shell"))?;

        match class.as_str() {
            "CabinetWClass" | "ExploreWClass" => explorer_selection(&shell_windows, fg),
            "Progman" | "WorkerW" => desktop_selection(&shell_windows),
            other => {
                log::info!("hotkey pressed in a window of class {other}");
                Err("Select files in File Explorer or on the desktop first, then press the shortcut.".into())
            }
        }
    }
}

/// A VARIANT holding a 32-bit integer (VT_I4).
fn variant_i4(value: i32) -> VARIANT {
    let mut var = VARIANT::default();
    unsafe {
        let inner = &mut *var.Anonymous.Anonymous;
        inner.vt = VT_I4;
        inner.Anonymous.lVal = value;
    }
    var
}

unsafe fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let len = GetClassNameW(hwnd, &mut buf);
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

unsafe fn explorer_selection(shell_windows: &IShellWindows, fg: HWND) -> Res<Selection> {
    // Win11 Explorer keeps every tab in one top-level window. The first
    // ShellTabWindowClass child is the tab currently shown.
    let active_tab = FindWindowExW(Some(fg), None, w!("ShellTabWindowClass"), PCWSTR::null()).ok();

    let count = shell_windows.Count().map_err(err("Couldn't list Explorer windows"))?;
    for i in 0..count {
        let Ok(disp) = shell_windows.Item(&variant_i4(i)) else { continue };
        let Ok(browser2) = disp.cast::<IWebBrowser2>() else { continue };
        let Ok(hwnd) = browser2.HWND() else { continue };
        if hwnd.0 != fg.0 as isize {
            continue;
        }
        let Ok(provider) = browser2.cast::<IServiceProvider>() else { continue };
        let Ok(browser) = provider.QueryService::<IShellBrowser>(&SID_STopLevelBrowser) else {
            continue;
        };
        if let Some(tab) = active_tab {
            match browser.GetWindow() {
                Ok(tab_hwnd) if tab_hwnd == tab => {}
                _ => continue,
            }
        }
        return Ok(Selection {
            source: "File Explorer".into(),
            paths: selected_paths(&browser)?,
        });
    }
    Err("Couldn't find the active Explorer tab".into())
}

unsafe fn desktop_selection(shell_windows: &IShellWindows) -> Res<Selection> {
    let location = variant_i4(CSIDL_DESKTOP as i32);
    let root = VARIANT::default();
    let mut hwnd: i32 = 0;
    let disp = shell_windows
        .FindWindowSW(&location, &root, SWC_DESKTOP, &mut hwnd, SWFO_NEEDDISPATCH)
        .map_err(err("Couldn't reach the desktop"))?;
    let provider: IServiceProvider = disp.cast().map_err(err("Desktop has no service provider"))?;
    let browser: IShellBrowser = provider
        .QueryService(&SID_STopLevelBrowser)
        .map_err(err("Couldn't reach the desktop view"))?;
    Ok(Selection {
        source: "Desktop".into(),
        paths: selected_paths(&browser)?,
    })
}

unsafe fn selected_paths(browser: &IShellBrowser) -> Res<Vec<String>> {
    let view = browser
        .QueryActiveShellView()
        .map_err(err("Couldn't read the folder view"))?;
    let folder_view: IFolderView2 = view.cast().map_err(err("Unsupported folder view"))?;

    // Fails (or returns nothing) when no item is selected.
    let Ok(items) = folder_view.GetSelection(false) else {
        return Ok(Vec::new());
    };
    let count = items.GetCount().map_err(err("Couldn't count the selection"))?;

    let mut paths = Vec::with_capacity(count as usize);
    let mut virtual_items = 0;
    for i in 0..count {
        let Ok(item) = items.GetItemAt(i) else { continue };
        // Virtual items (inside a ZIP opened in Explorer, the Recycle Bin, This PC,
        // a phone) have no file-system path; skip them.
        match item.GetDisplayName(SIGDN_FILESYSPATH) {
            Ok(p) => paths.push(pwstr_to_string(p)),
            Err(_) => virtual_items += 1,
        }
    }
    if paths.is_empty() && virtual_items > 0 {
        return Err("These items aren't ordinary files (inside a ZIP, the Recycle Bin or a phone, say). Copy or extract them to a folder first.".into());
    }
    Ok(paths)
}

unsafe fn pwstr_to_string(p: PWSTR) -> String {
    let s = p.to_string().unwrap_or_default();
    CoTaskMemFree(Some(p.0 as *const _));
    s
}

/// Makes sure the file under the pointer is selected, like a normal
/// right-click does: an unselected item becomes the only selection, a
/// selected item keeps the whole selection. Returns true if a file was there.
pub fn select_item_at(x: i32, y: i32) -> Res<bool> {
    // UI Automation clients work best on their own multithreaded apartment.
    std::thread::spawn(move || unsafe {
        let _com = ComGuard(CoInitializeEx(None, COINIT_MULTITHREADED).is_ok());
        let uia: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
            .map_err(err("Couldn't start UI Automation"))?;
        let walker = uia.ControlViewWalker().map_err(err("UI Automation walker"))?;
        let mut element = uia
            .ElementFromPoint(POINT { x, y })
            .map_err(err("Nothing under the pointer"))?;
        // The element under the pointer is often the file's name label; walk up to the list item.
        for _ in 0..6 {
            if element.CurrentControlType().ok() == Some(UIA_ListItemControlTypeId) {
                let pattern: IUIAutomationSelectionItemPattern = element
                    .GetCurrentPatternAs(UIA_SelectionItemPatternId)
                    .map_err(err("This item can't be selected"))?;
                let selected = pattern.CurrentIsSelected().map(|b| b.as_bool()).unwrap_or(false);
                if !selected {
                    pattern.Select().map_err(err("Couldn't select the file"))?;
                    // Give Explorer a moment to update its selection.
                    std::thread::sleep(std::time::Duration::from_millis(60));
                }
                return Ok(true);
            }
            match walker.GetParentElement(&element) {
                Ok(parent) => element = parent,
                Err(_) => break,
            }
        }
        Ok(false)
    })
    .join()
    .map_err(|_| "UI Automation crashed".to_string())?
}
