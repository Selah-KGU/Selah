use super::*;

pub(in crate::windows_native_agent) fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | ((g as u32) << 8) | ((b as u32) << 16)
}

pub(in crate::windows_native_agent) fn wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub(in crate::windows_native_agent) fn hwnd_from_raw(r: RawHwnd) -> HWND {
    r as HWND
}

pub(in crate::windows_native_agent) fn ease_out_quart(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(4)
}

pub(in crate::windows_native_agent) fn uuid_v4() -> String {
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-\
         {:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0],
        b[1],
        b[2],
        b[3],
        b[4],
        b[5],
        b[6],
        b[7],
        b[8],
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15],
    )
}

// ─ Font helpers ───────────────────────────────────────────────────────────────
fn make_font(px: i32, bold: bool) -> HGDIOBJ {
    let name = wide_null("Segoe UI");
    let weight = if bold {
        FW_BOLD as i32
    } else {
        FW_NORMAL as i32
    };
    unsafe {
        CreateFontW(
            -px,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            0,
            DEFAULT_QUALITY as u32,
            (DEFAULT_PITCH | FF_DONTCARE) as u32,
            name.as_ptr(),
        ) as HGDIOBJ
    }
}

pub(in crate::windows_native_agent) fn get_listen_font() -> HGDIOBJ {
    *CACHED_LISTEN_FONT.get_or_init(|| make_font(LISTEN_FONT_PX, true) as isize) as HGDIOBJ
}

pub(in crate::windows_native_agent) fn get_result_font() -> HGDIOBJ {
    *CACHED_RESULT_FONT.get_or_init(|| make_font(RESULT_FONT_PX, false) as isize) as HGDIOBJ
}

pub(in crate::windows_native_agent) fn get_notice_font() -> HGDIOBJ {
    *CACHED_NOTICE_FONT.get_or_init(|| make_font(NOTICE_FONT_PX, false) as isize) as HGDIOBJ
}

// ─ Thread-local background brush ─────────────────────────────────────────────
pub(in crate::windows_native_agent) unsafe fn get_bg_brush(color: u32) -> HGDIOBJ {
    TL_BG_BRUSH.with(|cell| {
        let (cached_color, cached_handle) = cell.get();
        if cached_color == color && cached_handle != 0 {
            return cached_handle as HGDIOBJ;
        }
        if cached_handle != 0 {
            DeleteObject(cached_handle as HGDIOBJ);
        }
        let h = CreateSolidBrush(color) as usize;
        cell.set((color, h));
        h as HGDIOBJ
    })
}

// ─ System helpers ─────────────────────────────────────────────────────────────
pub(in crate::windows_native_agent) fn work_area() -> RECT {
    let mut r = RECT::default();
    unsafe {
        if SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut r as *mut RECT).cast(), 0) != 0 {
            return r;
        }
    }
    RECT {
        left: 0,
        top: 0,
        right: unsafe { GetSystemMetrics(SM_CXSCREEN) },
        bottom: unsafe { GetSystemMetrics(SM_CYSCREEN) },
    }
}

pub(in crate::windows_native_agent) fn prefers_dark(app: &AppHandle) -> bool {
    let theme = app.state::<crate::ThemeState>();
    let mode = theme.0.lock().unwrap_or_else(|e| e.into_inner()).clone();
    match mode.as_str() {
        "light" => false,
        "dark" => true,
        _ => system_apps_use_dark_theme(),
    }
}

fn system_apps_use_dark_theme() -> bool {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE,
        REG_DWORD,
    };
    let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0"
        .encode_utf16()
        .collect();
    let mut hkey: HKEY = null_mut();
    if unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut hkey,
        )
    } != ERROR_SUCCESS
    {
        return true;
    }
    let vname: Vec<u16> = "AppsUseLightTheme\0".encode_utf16().collect();
    let mut data: u32 = 0;
    let mut data_size = std::mem::size_of::<u32>() as u32;
    let mut data_type: u32 = 0;
    let res = unsafe {
        RegQueryValueExW(
            hkey,
            vname.as_ptr(),
            null_mut(),
            &mut data_type,
            &mut data as *mut u32 as *mut u8,
            &mut data_size,
        )
    };
    let _ = unsafe { RegCloseKey(hkey) };
    if res != ERROR_SUCCESS || data_type != REG_DWORD {
        return true;
    }
    data == 0
}
