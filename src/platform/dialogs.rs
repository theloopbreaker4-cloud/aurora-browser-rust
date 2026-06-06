// dialogs.rs — Native OS dialogs (file picker, color picker, alert/confirm/prompt).
// Currently Win32 only; macOS/Linux can be added later behind cfg gates.
//
// Why not the COM-based IFileOpenDialog: GetOpenFileNameW is a 30-year-old API
// that is far simpler from Rust (no COM init, no IUnknown lifetime juggling),
// and its UX matches the modern shell dialog on Windows 10/11 since the OS
// auto-upgrades it.

#![cfg(all(windows, feature = "servo-engine"))]

use std::path::PathBuf;

/// One row of a popup menu shown by `popup_menu()`.
/// `Item { id }` is what's returned when the user picks it.
/// `Separator` draws a horizontal divider. `Group { label, items }` becomes a
/// disabled header followed by indented items (used for `<optgroup>`).
#[derive(Clone, Debug)]
pub enum PopupItem {
    Item { id: u32, label: String, checked: bool, disabled: bool },
    Separator,
    Group { label: String, items: Vec<PopupItem> },
}

/// A pattern accepted by `<input type="file" accept="...">`.
/// `description` is shown in the filter dropdown; `extensions` are bare ext strings (no leading dot).
#[derive(Clone, Debug)]
pub struct FileFilter {
    pub description: String,
    pub extensions: Vec<String>,
}

/// Show a native Open File dialog and return the selected path(s).
/// Returns `None` if the user cancelled, otherwise a non-empty Vec of paths.
///
/// `parent_hwnd` is the owning window so the dialog is modal to Aurora.
/// `multi` enables multi-selection (`<input multiple>`).
/// `filters` becomes the dropdown filter list; an "All files (*.*)" entry is always appended.
pub fn open_file_dialog(
    parent_hwnd: isize,
    multi: bool,
    filters: &[FileFilter],
) -> Option<Vec<PathBuf>> {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, OFN_ALLOWMULTISELECT, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY,
        OFN_NOCHANGEDIR, OPENFILENAMEW,
    };

    // Build the Windows-style filter string: pairs of NUL-terminated UTF-16 strings,
    // double-NUL terminated. Format: "Images\0*.png;*.jpg\0All Files\0*.*\0\0".
    let mut filter_buf: Vec<u16> = Vec::with_capacity(256);
    for f in filters {
        push_utf16_z(&mut filter_buf, &f.description);
        let pattern = if f.extensions.is_empty() {
            "*.*".to_string()
        } else {
            f.extensions
                .iter()
                .map(|e| format!("*.{}", e.trim_start_matches('.')))
                .collect::<Vec<_>>()
                .join(";")
        };
        push_utf16_z(&mut filter_buf, &pattern);
    }
    // Always add an "All Files" fallback so the user can override the filter.
    push_utf16_z(&mut filter_buf, "All Files (*.*)");
    push_utf16_z(&mut filter_buf, "*.*");
    filter_buf.push(0); // double-NUL terminator

    // GetOpenFileNameW writes into this buffer. For multi-select it returns
    // "directory\0file1\0file2\0\0" — needs to be large enough.
    let mut file_buf: Vec<u16> = vec![0u16; if multi { 65536 } else { 4096 }];

    let mut flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_HIDEREADONLY | OFN_NOCHANGEDIR;
    if multi {
        flags |= OFN_ALLOWMULTISELECT;
    }

    let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = parent_hwnd as HWND;
    ofn.lpstrFilter = filter_buf.as_ptr();
    ofn.lpstrFile = file_buf.as_mut_ptr();
    ofn.nMaxFile = file_buf.len() as u32;
    ofn.Flags = flags;

    let ok = unsafe { GetOpenFileNameW(&mut ofn) };
    if ok == 0 {
        return None;
    }

    Some(parse_file_buf(&file_buf, multi))
}

/// Push a UTF-16 string + NUL terminator onto a buffer.
fn push_utf16_z(out: &mut Vec<u16>, s: &str) {
    out.extend(s.encode_utf16());
    out.push(0);
}

/// Show a native Win32 popup menu at the given screen coordinates.
/// `screen_x` / `screen_y` are absolute screen pixels (use ClientToScreen first
/// if you have window-relative coords). Returns `Some(id)` of the chosen Item,
/// or `None` if dismissed. `parent_hwnd` owns the menu so input routing works.
unsafe fn append_popup_item(
    menu: windows_sys::Win32::UI::WindowsAndMessaging::HMENU,
    item: &PopupItem,
) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, MF_CHECKED, MF_DISABLED, MF_GRAYED, MF_POPUP, MF_SEPARATOR,
        MF_STRING,
    };
    match item {
        PopupItem::Separator => {
            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        }
        PopupItem::Item {
            id,
            label,
            checked,
            disabled,
        } => {
            let mut flags = MF_STRING;
            if *checked {
                flags |= MF_CHECKED;
            }
            if *disabled {
                flags |= MF_DISABLED | MF_GRAYED;
            }
            let w = utf16_z(label);
            AppendMenuW(menu, flags, (*id as usize) + 1, w.as_ptr());
        }
        PopupItem::Group { label, items } => {
            let sub = CreatePopupMenu();
            if sub.is_null() {
                return;
            }
            for it in items {
                append_popup_item(sub, it);
            }
            let w = utf16_z(label);
            AppendMenuW(menu, MF_POPUP, sub as usize, w.as_ptr());
        }
    }
}

fn utf16_z(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn popup_menu(
    parent_hwnd: isize,
    screen_x: i32,
    screen_y: i32,
    items: &[PopupItem],
) -> Option<u32> {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreatePopupMenu, DestroyMenu, SetForegroundWindow, TrackPopupMenu, TPM_LEFTALIGN,
        TPM_RETURNCMD, TPM_TOPALIGN,
    };

    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return None;
        }
        for it in items {
            append_popup_item(menu, it);
        }
        // Per MSDN: must SetForegroundWindow before TrackPopupMenu so the menu
        // dismisses correctly when the user clicks elsewhere.
        SetForegroundWindow(parent_hwnd as HWND);
        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN,
            screen_x,
            screen_y,
            0,
            parent_hwnd as HWND,
            std::ptr::null(),
        );
        DestroyMenu(menu);
        if cmd == 0 {
            None
        } else {
            // We added 1 to all IDs in append_popup_item; reverse here.
            Some(cmd as u32 - 1)
        }
    }
}

/// Show the native Win32 color picker. Returns the chosen color as (r, g, b)
/// 0-255, or `None` if cancelled. `initial` pre-selects a color.
pub fn pick_color(parent_hwnd: isize, initial: Option<(u8, u8, u8)>) -> Option<(u8, u8, u8)> {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::Controls::Dialogs::{
        ChooseColorW, CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW,
    };

    // Custom palette buffer — required by ChooseColorW even if unused.
    let mut custom: [u32; 16] = [0; 16];
    let initial_rgb = initial
        .map(|(r, g, b)| u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16))
        .unwrap_or(0);

    let mut cc: CHOOSECOLORW = unsafe { std::mem::zeroed() };
    cc.lStructSize = std::mem::size_of::<CHOOSECOLORW>() as u32;
    cc.hwndOwner = parent_hwnd as HWND;
    cc.rgbResult = initial_rgb;
    cc.lpCustColors = custom.as_mut_ptr();
    cc.Flags = CC_FULLOPEN | CC_RGBINIT;

    let ok = unsafe { ChooseColorW(&mut cc) };
    if ok == 0 {
        return None;
    }

    let r = (cc.rgbResult & 0xFF) as u8;
    let g = ((cc.rgbResult >> 8) & 0xFF) as u8;
    let b = ((cc.rgbResult >> 16) & 0xFF) as u8;
    Some((r, g, b))
}

/// Type of script-initiated simple dialog.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SimpleDialogKind {
    Alert,
    Confirm,
    Prompt,
}

/// Outcome of a simple dialog.
#[derive(Debug)]
pub enum SimpleDialogResult {
    /// Alert dismissed (only outcome).
    Acknowledged,
    /// Confirm: true=OK, false=Cancel.
    Confirmed(bool),
    /// Prompt: Some(text) on OK, None on Cancel.
    Prompted(Option<String>),
}

/// Show a script-initiated dialog (alert/confirm/prompt).
/// Title is fixed to "Aurora" so the user cannot mistake it for browser chrome.
/// For `Prompt`, the implementation falls back to a fake "OK with empty text"
/// because Win32 has no native prompt() — we'll wire a proper input dialog later.
pub fn simple_dialog(
    parent_hwnd: isize,
    kind: SimpleDialogKind,
    message: &str,
    _default: Option<&str>,
) -> SimpleDialogResult {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDOK, MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK, MB_OKCANCEL,
    };

    let title: Vec<u16> = "Aurora".encode_utf16().chain(std::iter::once(0)).collect();
    let body: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();

    match kind {
        SimpleDialogKind::Alert => {
            unsafe {
                MessageBoxW(
                    parent_hwnd as HWND,
                    body.as_ptr(),
                    title.as_ptr(),
                    MB_OK | MB_ICONINFORMATION,
                );
            }
            SimpleDialogResult::Acknowledged
        }
        SimpleDialogKind::Confirm => {
            let r = unsafe {
                MessageBoxW(
                    parent_hwnd as HWND,
                    body.as_ptr(),
                    title.as_ptr(),
                    MB_OKCANCEL | MB_ICONQUESTION,
                )
            };
            SimpleDialogResult::Confirmed(r == IDOK as i32)
        }
        SimpleDialogKind::Prompt => {
            // TODO: real prompt UI. For now show OKCANCEL as a confirmation
            // that the message was seen, returning empty string on OK.
            let r = unsafe {
                MessageBoxW(
                    parent_hwnd as HWND,
                    body.as_ptr(),
                    title.as_ptr(),
                    MB_OKCANCEL | MB_ICONQUESTION,
                )
            };
            if r == IDOK as i32 {
                SimpleDialogResult::Prompted(Some(String::new()))
            } else {
                SimpleDialogResult::Prompted(None)
            }
        }
    }
}

/// Decode the GetOpenFileNameW result buffer.
/// Single-select: just the full path, NUL-terminated.
/// Multi-select: "directory\0file1\0file2\0...\0\0".
/// Quirk: if the user picks one file in multi-select mode, Windows still returns
/// just the full path (no directory split), so we handle that case.
fn parse_file_buf(buf: &[u16], multi: bool) -> Vec<PathBuf> {
    // Find first NUL.
    let first_nul = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let first = String::from_utf16_lossy(&buf[..first_nul]);

    if !multi {
        return vec![PathBuf::from(first)];
    }

    // Look at the next slice after first NUL — if it's empty, single file in multi mode.
    let rest = &buf[first_nul + 1..];
    let next_nul = rest.iter().position(|&c| c == 0).unwrap_or(0);
    if next_nul == 0 {
        // Only one file selected — `first` is the full path.
        return vec![PathBuf::from(first)];
    }

    // Multi-file: `first` is the directory, subsequent NUL-terminated entries are filenames.
    let dir = PathBuf::from(first);
    let mut files = Vec::new();
    let mut cursor = 0usize;
    while cursor < rest.len() {
        let end = rest[cursor..]
            .iter()
            .position(|&c| c == 0)
            .map(|p| cursor + p)
            .unwrap_or(rest.len());
        if end == cursor {
            break; // double-NUL = end of list
        }
        let name = String::from_utf16_lossy(&rest[cursor..end]);
        files.push(dir.join(name));
        cursor = end + 1;
    }

    files
}
