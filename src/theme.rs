//! カラーテーマ・ブラシ・ダークモードサポート。
//!
//! Windows の UxTheme / Dwm を経由したタイトルバー制御、
//! オーナードローによるカスタムメニュー描画を含む。

// Theme colors (COLORREF = 0x00BBGGRR)
pub const CLR_DARK_BG: u32 = 0x00202020;
pub const CLR_DARK_EDIT: u32 = 0x002D2D2D;
pub const CLR_DARK_FG: u32 = 0x00F0F0F0;
pub const CLR_DARK_LINE: u32 = 0x00A0A0A0; // TreeView hierarchy lines (visible on dark bg)
pub const CLR_LIGHT_BG: u32 = 0x00F0F0F0;
pub const CLR_LIGHT_EDIT: u32 = 0x00FFFFFF;
pub const CLR_LIGHT_FG: u32 = 0x00000000;
pub const CLR_LIGHT_LINE: u32 = 0x00707070; // TreeView hierarchy lines (visible on light bg)

pub struct ThemeBrushes {
    pub dark_bg: windows::Win32::Graphics::Gdi::HBRUSH,
    pub dark_edit: windows::Win32::Graphics::Gdi::HBRUSH,
    pub light_bg: windows::Win32::Graphics::Gdi::HBRUSH,
    pub light_edit: windows::Win32::Graphics::Gdi::HBRUSH,
}

impl Default for ThemeBrushes {
    fn default() -> Self {
        Self::new()
    }
}

impl ThemeBrushes {
    pub fn new() -> Self {
        use windows::Win32::Graphics::Gdi::CreateSolidBrush;
        use windows::Win32::Foundation::COLORREF;
        unsafe {
            Self {
                dark_bg: CreateSolidBrush(COLORREF(CLR_DARK_BG)),
                dark_edit: CreateSolidBrush(COLORREF(CLR_DARK_EDIT)),
                light_bg: CreateSolidBrush(COLORREF(CLR_LIGHT_BG)),
                light_edit: CreateSolidBrush(COLORREF(CLR_LIGHT_EDIT)),
            }
        }
    }

    pub fn bg(&self, dark: bool) -> windows::Win32::Graphics::Gdi::HBRUSH {
        if dark { self.dark_bg } else { self.light_bg }
    }

    pub fn edit(&self, dark: bool) -> windows::Win32::Graphics::Gdi::HBRUSH {
        if dark { self.dark_edit } else { self.light_edit }
    }
}

// Undocumented UAH (User32 Aero Hook) structures for dark menu bar
pub const WM_UAHDRAWMENU: u32 = 0x0091;
pub const WM_UAHDRAWMENUITEM: u32 = 0x0092;
pub const WM_UAHMEASUREMENUITEM: u32 = 0x0094;

#[repr(C)]
pub struct UAHMENU {
    pub hmenu: windows::Win32::UI::WindowsAndMessaging::HMENU,
    pub hdc: windows::Win32::Graphics::Gdi::HDC,
    pub dw_flags: u32,
}

#[repr(C)]
pub struct UAHMENUITEMMETRICS {
    pub cx: u32,
    pub cy: u32,
}

#[repr(C)]
pub struct UAHMENUPOPUPMETRICS {
    pub cx: u32,
    pub cy: u32,
}

#[repr(C)]
pub struct UAHMENUITEM {
    pub i_position: i32,
    pub umim: UAHMENUITEMMETRICS,
    pub umpm: UAHMENUPOPUPMETRICS,
}

#[repr(C)]
pub struct UAHDRAWMENUITEM {
    pub dis: windows::Win32::UI::Controls::DRAWITEMSTRUCT,
    pub um: UAHMENU,
    pub umi: UAHMENUITEM,
}

#[repr(C)]
pub struct UAHMEASUREMENUITEM {
    pub mis: windows::Win32::UI::Controls::MEASUREITEMSTRUCT,
    pub um: UAHMENU,
    pub umi: UAHMENUITEM,
}

/// uxtheme dark-mode helpers (Win10 1809+ undocumented ordinals)
pub fn uxtheme_set_app_mode(dark: bool) {
    use windows::Win32::System::LibraryLoader::{LoadLibraryW, GetProcAddress};
    use windows::core::{w, PCSTR};
    unsafe {
        let Ok(lib) = LoadLibraryW(w!("uxtheme.dll")) else { return };
        // 135: SetPreferredAppMode — 2=ForceDark, 3=ForceLight
        if let Some(proc) = GetProcAddress(lib, PCSTR(135usize as *const u8)) {
            let func: extern "system" fn(i32) -> i32 = std::mem::transmute(proc);
            let _ = func(if dark { 2 } else { 3 });
        }
        // 104: RefreshImmersiveColorPolicyState
        if let Some(proc) = GetProcAddress(lib, PCSTR(104usize as *const u8)) {
            let func: extern "system" fn() = std::mem::transmute(proc);
            func();
        }
        // 136: FlushMenuThemes
        if let Some(proc) = GetProcAddress(lib, PCSTR(136usize as *const u8)) {
            let func: extern "system" fn() = std::mem::transmute(proc);
            func();
        }
    }
}

pub fn allow_dark_mode_for_window(hwnd: windows::Win32::Foundation::HWND, allow: bool) {
    use windows::Win32::System::LibraryLoader::{LoadLibraryW, GetProcAddress};
    use windows::core::{w, PCSTR};
    unsafe {
        let Ok(lib) = LoadLibraryW(w!("uxtheme.dll")) else { return };
        // 133: AllowDarkModeForWindow(HWND, BOOL)
        if let Some(proc) = GetProcAddress(lib, PCSTR(133usize as *const u8)) {
            let func: extern "system" fn(
                windows::Win32::Foundation::HWND,
                windows::Win32::Foundation::BOOL,
            ) -> windows::Win32::Foundation::BOOL = std::mem::transmute(proc);
            let _ = func(hwnd, windows::Win32::Foundation::BOOL::from(allow));
        }
    }
}

pub fn apply_dark_titlebar(hwnd: windows::Win32::Foundation::HWND, dark: bool) {
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
    let dark_val: i32 = if dark { 1 } else { 0 };
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark_val as *const i32 as *const _,
            std::mem::size_of::<i32>() as u32,
        );
    }
    allow_dark_mode_for_window(hwnd, dark);
}

pub fn set_control_theme(hwnd: windows::Win32::Foundation::HWND, dark: bool) {
    use windows::Win32::UI::Controls::SetWindowTheme;
    use windows::core::w;
    unsafe {
        if dark {
            let _ = SetWindowTheme(hwnd, w!("DarkMode_Explorer"), None);
        } else {
            let _ = SetWindowTheme(hwnd, w!("Explorer"), None);
        }
        allow_dark_mode_for_window(hwnd, dark);
    }
}

pub fn set_scrollbar_theme(hwnd: windows::Win32::Foundation::HWND, dark: bool) {
    use windows::Win32::UI::Controls::SetWindowTheme;
    use windows::core::w;
    unsafe {
        if dark {
            let _ = SetWindowTheme(hwnd, w!("DarkMode_Explorer"), w!("ScrollBar"));
        } else {
            let _ = SetWindowTheme(hwnd, w!("Explorer"), w!("ScrollBar"));
        }
        allow_dark_mode_for_window(hwnd, dark);
    }
}

pub fn clear_control_theme(hwnd: windows::Win32::Foundation::HWND) {
    use windows::Win32::UI::Controls::SetWindowTheme;
    use windows::core::w;
    unsafe {
        let _ = SetWindowTheme(hwnd, w!(""), w!(""));
    }
}

pub fn set_edit_theme(hwnd: windows::Win32::Foundation::HWND, dark: bool) {
    use windows::Win32::UI::Controls::SetWindowTheme;
    use windows::core::w;
    unsafe {
        if dark {
            // Disable visual styles so WM_CTLCOLOREDIT colors apply
            let _ = SetWindowTheme(hwnd, w!(""), w!(""));
        } else {
            let _ = SetWindowTheme(hwnd, w!("Explorer"), None);
        }
    }
}

pub fn set_combo_theme(hwnd: windows::Win32::Foundation::HWND, dark: bool) {
    use windows::Win32::UI::Controls::SetWindowTheme;
    use windows::core::{w, PCWSTR};
    unsafe {
        if dark {
            // DarkMode_CFD works for ComboBox on Win10/11; fallback to unthemed
            if SetWindowTheme(hwnd, w!("DarkMode_CFD"), None).is_err() {
                let _ = SetWindowTheme(hwnd, PCWSTR::null(), PCWSTR::null());
            }
        } else {
            let _ = SetWindowTheme(hwnd, w!("Explorer"), None);
        }
    }
}

/// Darken menus via immersive theme only.
/// Do NOT set MNS_NOCHECK / MIM_BACKGROUND on the menubar itself (breaks menu clicks).
pub fn apply_menu_theme(hwnd: windows::Win32::Foundation::HWND, _brushes: &ThemeBrushes, dark: bool) {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetMenu, GetSubMenu, GetMenuItemCount, SetMenuInfo, DrawMenuBar, MENUINFO, MIM_STYLE,
        MNS_NOCHECK,
    };
    // Immersive dark menus (bg + text) via uxtheme
    uxtheme_set_app_mode(dark);
    allow_dark_mode_for_window(hwnd, dark);

    // Keep only the checkmark gutter removal on popup submenus
    unsafe {
        let hmenu = GetMenu(hwnd);
        if hmenu.is_invalid() {
            return;
        }
        let count = GetMenuItemCount(hmenu);
        for i in 0..count {
            let sub = GetSubMenu(hmenu, i);
            if !sub.is_invalid() {
                let sub_mi = MENUINFO {
                    cbSize: std::mem::size_of::<MENUINFO>() as u32,
                    fMask: MIM_STYLE,
                    dwStyle: MNS_NOCHECK,
                    cyMax: 0,
                    hbrBack: windows::Win32::Graphics::Gdi::HBRUSH::default(),
                    dwContextHelpID: 0,
                    dwMenuData: 0,
                };
                let _ = SetMenuInfo(sub, &sub_mi);
            }
        }
        let _ = DrawMenuBar(hwnd);
    }
}

pub fn uah_draw_menu_nc_bottom_line(hwnd: windows::Win32::Foundation::HWND, dark: bool, brushes: &ThemeBrushes) {
    if !dark { return; }
    use windows::Win32::UI::WindowsAndMessaging::{GetMenuBarInfo, OBJID_MENU, MENUBARINFO, GetWindowRect};
    use windows::Win32::Graphics::Gdi::{GetWindowDC, ReleaseDC, FillRect, ClientToScreen};
    use windows::Win32::Foundation::RECT;

    unsafe {
        let mut mbi = MENUBARINFO {
            cbSize: std::mem::size_of::<MENUBARINFO>() as u32,
            ..Default::default()
        };
        if GetMenuBarInfo(hwnd, OBJID_MENU, 0, &mut mbi).is_err() {
            return;
        }

        let mut pt = windows::Win32::Foundation::POINT::default();
        let _ = ClientToScreen(hwnd, &mut pt);

        let mut rc_window = RECT::default();
        let _ = GetWindowRect(hwnd, &mut rc_window);

        let mut rc_client = RECT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rc_client);

        let client_top_in_window = pt.y - rc_window.top;

        let client_left_in_window = pt.x - rc_window.left;
        let rc_line = RECT {
            left: client_left_in_window,
            right: client_left_in_window + rc_client.right,
            top: client_top_in_window - 1,
            bottom: client_top_in_window,
        };

        let hdc = GetWindowDC(hwnd);
        let _ = FillRect(hdc, &rc_line, brushes.bg(true));
        let _ = ReleaseDC(hwnd, hdc);
    }
}
