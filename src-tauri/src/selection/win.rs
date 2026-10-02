//! Windows adapter: reads the selection from File Explorer or the desktop
//! through the Shell's COM interfaces.
//!
//! Flow: foreground window → matching entry in `ShellWindows` (for Win11
//! tabs, the entry whose browser window is the active tab) → its shell view
//! → `IFolderView2::GetSelection` → file-system paths.

use super::dialog;
use super::Selection;
use std::path::PathBuf;
use ::windows::core::{w, Interface, PCWSTR, PWSTR};
use ::windows::Win32::Foundation::HWND;
use ::windows::Win32::Foundation::POINT;
use ::windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IServiceProvider,
    CLSCTX_ALL, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED,
};
use ::windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationSelectionItemPattern, IUIAutomationSelectionPattern,
    TreeScope_Descendants, UIA_ControlTypePropertyId, UIA_ListControlTypeId, UIA_ListItemControlTypeId,
    UIA_SelectionItemPatternId, UIA_SelectionPatternId,
};
use ::windows::Win32::Foundation::{LPARAM, WPARAM};
use ::windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music, FOLDERID_Pictures,
    FOLDERID_Profile, FOLDERID_Videos, IShellItem, SHGetKnownFolderItem, KF_FLAG_DEFAULT, SIGDN_NORMALDISPLAY,
};
use ::windows::Win32::System::Variant::{VARIANT, VT_I4};
use ::windows::Win32::UI::Shell::{
    IFolderView2, IShellBrowser, IShellWindows, IWebBrowser2, ShellWindows, CSIDL_DESKTOP,
    SID_STopLevelBrowser, SIGDN_FILESYSPATH, SWC_DESKTOP, SWFO_NEEDDISPATCH,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, FindWindowExW, GetClassNameW, GetDlgCtrlID, GetForegroundWindow, GetParent,
    SendMessageTimeoutW, SMTO_ABORTIFHUNG, WM_GETTEXT,
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
            "#32770" => dialog_selection(fg),
            other => {
                log::info!("hotkey pressed in a window of class {other}");
                Err("Hold Alt and right-click a file in File Explorer or on the desktop. (Or select files first, then press your shortcut.)".into())
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
        ..Default::default()
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
        ..Default::default()
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

// ---------------------------------------------------------------------------
// Open and Save dialogs (another app's window: read from the outside).

/// Every child window, at any depth.
unsafe fn descendants(parent: HWND) -> Vec<HWND> {
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> ::windows::core::BOOL {
        let v = &mut *(lparam.0 as *mut Vec<HWND>);
        v.push(hwnd);
        true.into()
    }
    let mut v: Vec<HWND> = Vec::new();
    let _ = EnumChildWindows(Some(parent), Some(collect), LPARAM(&mut v as *mut _ as isize));
    v
}

/// A control's text, also in another app (WM_GETTEXT is passed across processes).
unsafe fn text_of(hwnd: HWND) -> String {
    let mut buf = vec![0u16; 2048];
    let mut copied: usize = 0;
    let r = SendMessageTimeoutW(
        hwnd,
        WM_GETTEXT,
        WPARAM(buf.len()),
        LPARAM(buf.as_mut_ptr() as isize),
        SMTO_ABORTIFHUNG,
        500,
        Some(&mut copied),
    );
    if r.0 == 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..copied.min(buf.len())])
}

/// The folder the dialog shows, from its address bar.
unsafe fn dialog_folder(all: &[HWND]) -> Option<PathBuf> {
    let bar = all.iter().copied().find(|h| {
        class_name(*h) == "ToolbarWindow32" && GetParent(*h).map(|p| class_name(p) == "Breadcrumb Parent").unwrap_or(false)
    })?;
    let shown = dialog::parse_address(&text_of(bar));
    let p = PathBuf::from(&shown);
    if p.is_absolute() && p.is_dir() {
        return Some(p);
    }
    // Known folders show their name ("Downloads", "Desktop"), in the user's language.
    for id in [&FOLDERID_Desktop, &FOLDERID_Documents, &FOLDERID_Downloads, &FOLDERID_Pictures, &FOLDERID_Music, &FOLDERID_Videos, &FOLDERID_Profile] {
        let Ok(item) = SHGetKnownFolderItem::<IShellItem>(id, KF_FLAG_DEFAULT, None) else { continue };
        let Ok(name) = item.GetDisplayName(SIGDN_NORMALDISPLAY) else { continue };
        if pwstr_to_string(name).eq_ignore_ascii_case(&shown) {
            if let Ok(path) = item.GetDisplayName(SIGDN_FILESYSPATH) {
                return Some(PathBuf::from(pwstr_to_string(path)));
            }
        }
    }
    log::info!("file dialog: couldn't turn the address {shown:?} into a folder");
    None
}

/// Names of the items selected in the dialog's file list (UI Automation).
fn dialog_selected_names(view: isize) -> Vec<String> {
    std::thread::spawn(move || unsafe {
        let _com = ComGuard(CoInitializeEx(None, COINIT_MULTITHREADED).is_ok());
        let Ok(uia) = CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER) else { return Vec::new() };
        let Ok(root) = uia.ElementFromHandle(HWND(view as *mut core::ffi::c_void)) else { return Vec::new() };
        let Ok(cond) = uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &variant_i4(UIA_ListControlTypeId.0)) else {
            return Vec::new();
        };
        let Ok(list) = root.FindFirst(TreeScope_Descendants, &cond) else { return Vec::new() };
        let Ok(pattern) = list.GetCurrentPatternAs::<IUIAutomationSelectionPattern>(UIA_SelectionPatternId) else {
            return Vec::new();
        };
        let Ok(selected) = pattern.GetCurrentSelection() else { return Vec::new() };
        let n = selected.Length().unwrap_or(0);
        (0..n)
            .filter_map(|i| selected.GetElement(i).ok())
            .filter_map(|e| e.CurrentName().ok().map(|b| b.to_string()))
            .filter(|s| !s.is_empty())
            .collect()
    })
    .join()
    .unwrap_or_default()
}

/// A control anywhere inside the dialog by its id (the modern dialog nests
/// them a few windows deep, so GetDlgItem on the dialog alone misses them).
unsafe fn control(all: &[HWND], id: i32) -> Option<HWND> {
    all.iter().copied().find(|h| GetDlgCtrlID(*h) == id)
}

/// The "File name" box and the "Save as type" list. Classic dialogs use the
/// standard ids; the modern one puts the name box (an Edit, id 1001) in an
/// unnumbered ComboBox, and the type list is the ComboBox showing "(*.ext)".
unsafe fn dialog_name_and_type(all: &[HWND]) -> (String, String) {
    const ADDRESS: i32 = 0xA205;
    let in_combo = |h: HWND| GetParent(h).map(|p| class_name(p) == "ComboBox" && GetDlgCtrlID(p) != ADDRESS).unwrap_or(false);
    let name_box = control(all, 0x47C)
        .and_then(|c| if class_name(c) == "Edit" { Some(c) } else { descendants(c).into_iter().find(|h| class_name(*h) == "Edit") })
        .or_else(|| all.iter().copied().find(|h| class_name(*h) == "Edit" && GetDlgCtrlID(*h) == 0x3E9))
        .or_else(|| control(all, 0x480))
        .or_else(|| all.iter().copied().find(|h| class_name(*h) == "Edit" && GetDlgCtrlID(*h) != ADDRESS && in_combo(*h)));
    let name = name_box.map(|h| text_of(h)).unwrap_or_default();
    let kind = control(all, 0x470)
        .map(|h| text_of(h))
        .filter(|s| !s.is_empty())
        .or_else(|| {
            all.iter()
                .filter(|h| class_name(**h) == "ComboBox")
                .map(|h| text_of(*h))
                .find(|s| s.contains("*."))
        })
        .unwrap_or_default();
    (name, kind)
}

/// An Open or Save dialog: the files selected in its list, else the names in
/// "File name". In a Save dialog whose file doesn't exist yet, the conversion
/// waits for it (`pending_dialog`).
unsafe fn dialog_selection(dlg: HWND) -> Res<Selection> {
    let all = descendants(dlg);
    let Some(view) = all.iter().copied().find(|h| class_name(*h) == "SHELLDLL_DefView") else {
        return Err("Select files in File Explorer, on the desktop or in an Open or Save window, then press the shortcut.".into());
    };
    let folder = dialog_folder(&all).ok_or("Convertino can't tell which folder this window shows. Open a normal folder (not This PC or a library) and try again.")?;
    let selected = dialog_selected_names(view.0 as isize);
    let paths = dialog::resolve(&folder, &selected);
    if !paths.is_empty() {
        return Ok(Selection { source: "File dialog".into(), paths: strings(paths), ..Default::default() });
    }
    let (typed, kind) = dialog_name_and_type(&all);
    log::info!("file dialog: folder {}, selected {selected:?}, name box {typed:?}, type {kind:?}", folder.display());
    let names = dialog::parse_names(&typed);
    let paths = dialog::resolve(&folder, &names);
    if !paths.is_empty() {
        return Ok(Selection { source: "File dialog".into(), paths: strings(paths), ..Default::default() });
    }
    if let [one] = names.as_slice() {
        if let Some(target) = dialog::save_target(&folder, one, &kind) {
            log::info!("file dialog: {} doesn't exist yet; converting it once it's saved", target.display());
            return Ok(Selection {
                source: "Save dialog".into(),
                paths: strings(vec![target]),
                pending_dialog: Some(dlg.0 as isize),
            });
        }
    }
    Err("Select a file in this window, or type a file name, then press the shortcut.".into())
}

fn strings(paths: Vec<PathBuf>) -> Vec<String> {
    paths.into_iter().map(|p| p.to_string_lossy().into_owned()).collect()
}

/// Whether the window still exists (a Save dialog closes once the file is saved).
pub fn window_open(hwnd: isize) -> bool {
    unsafe { ::windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(HWND(hwnd as *mut core::ffi::c_void))).as_bool() }
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
