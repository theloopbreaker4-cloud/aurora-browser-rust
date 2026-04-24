// dialogs.rs — Native OS dialogs (file picker, color picker, alert/confirm/prompt).
// Currently Win32 only; macOS/Linux can be added later behind cfg gates.
//
// Why not the COM-based IFileOpenDialog: GetOpenFileNameW is a 30-year-old API
// that is far simpler from Rust (no COM init, no IUnknown lifetime juggling),
// and its UX matches the modern shell dialog on Windows 10/11 since the OS
// auto-upgrades it.

#![cfg(all(windows, feature = "servo-engine"))]

use std::path::PathBuf;

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
