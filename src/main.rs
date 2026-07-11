#![windows_subsystem = "windows"]

extern crate native_windows_gui as nwg;
extern crate native_windows_derive as nwd;

use jgrep3::custom_draw::*;
use jgrep3::drives;
use jgrep3::search;
use jgrep3::theme::*;

use nwd::NwgUi;
use nwg::NativeUi;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use settings_dialog_ui::SettingsDialogUi;

use regex::RegexBuilder;
use search::SearchStatus;

pub struct SearchState {
    pub cancel_token: Arc<AtomicBool>,
    pub receiver: Receiver<SearchStatus>,
    #[allow(dead_code)]
    pub thread_handle: std::thread::JoinHandle<()>,
}



// Helper to query Windows registry for System theme mode
fn is_system_dark_mode() -> bool {
    use windows::Win32::System::Registry::{RegOpenKeyExW, RegQueryValueExW, HKEY_CURRENT_USER, KEY_READ};
    use windows::core::w;

    let mut hkey = windows::Win32::System::Registry::HKEY::default();
    let subkey = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");

    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, subkey, 0, KEY_READ, &mut hkey).is_ok() {
            let mut data = 0u32;
            let mut data_len = std::mem::size_of::<u32>() as u32;

            let res = RegQueryValueExW(
                hkey,
                w!("AppsUseLightTheme"),
                None,
                None,
                Some(&mut data as *mut u32 as *mut _),
                Some(&mut data_len),
            );

            let _ = windows::Win32::System::Registry::RegCloseKey(hkey);

            if res.is_ok() && data == 0 {
                return true; // 0 means Dark Mode
            }
        }
    }
    false
}

const CONFIG_FILE_NAME: &str = "JGrep3.toml";
const CONFIG_APP_DIR: &str = "JGrep3";
const HISTORY_MAX: usize = 100;

/// Available UI fonts (Japanese Windows)
const UI_FONT_FAMILIES: &[&str] = &[
    "Meiryo UI",
    "Yu Gothic UI",
    "MS UI Gothic",
    "Segoe UI",
    "Meiryo",
    "Yu Gothic",
    "Consolas",
];
const DEFAULT_UI_FONT_FAMILY: &str = "Meiryo UI";
const DEFAULT_UI_FONT_SIZE: u32 = 16;

/// Result list fonts (prefer monospace for code/search hits)
const LIST_FONT_FAMILIES: &[&str] = &[
    "Cascadia Code",
    "Cascadia Mono",
    "Consolas",
    "Courier New",
    "Meiryo UI",
    "MS Gothic",
    "Segoe UI",
];
const DEFAULT_LIST_FONT_FAMILY: &str = "Cascadia Code";
const DEFAULT_LIST_FONT_SIZE: u32 = 14;

/// Font size choices shown in settings
const FONT_SIZES: &[u32] = &[10, 11, 12, 13, 14, 15, 16, 18, 20, 22, 24];

fn font_size_index(size: u32) -> usize {
    FONT_SIZES
        .iter()
        .position(|&s| s == size)
        .unwrap_or_else(|| {
            // nearest
            FONT_SIZES
                .iter()
                .enumerate()
                .min_by_key(|(_, s)| (**s as i32 - size as i32).unsigned_abs())
                .map(|(i, _)| i)
                .unwrap_or(0)
        })
}


fn default_font_family() -> String {
    DEFAULT_UI_FONT_FAMILY.to_string()
}

fn default_font_size() -> u32 {
    DEFAULT_UI_FONT_SIZE
}

fn default_list_font_family() -> String {
    DEFAULT_LIST_FONT_FAMILY.to_string()
}

fn default_list_font_size() -> u32 {
    DEFAULT_LIST_FONT_SIZE
}

const DEFAULT_TREE_WIDTH: u32 = 240;
const DEFAULT_COL_FILENAME_WIDTH: u32 = 350;
const DEFAULT_COL_LINE_WIDTH: u32 = 60;
const DEFAULT_COL_CONTENT_WIDTH: u32 = 450;
const DEFAULT_WINDOW_WIDTH: u32 = 1000;
const DEFAULT_WINDOW_HEIGHT: u32 = 680;

fn default_tree_width() -> u32 {
    DEFAULT_TREE_WIDTH
}
fn default_col_filename_width() -> u32 {
    DEFAULT_COL_FILENAME_WIDTH
}
fn default_col_line_width() -> u32 {
    DEFAULT_COL_LINE_WIDTH
}
fn default_col_content_width() -> u32 {
    DEFAULT_COL_CONTENT_WIDTH
}
fn default_window_width() -> u32 {
    DEFAULT_WINDOW_WIDTH
}
fn default_window_height() -> u32 {
    DEFAULT_WINDOW_HEIGHT
}

/// Physical pixels → logical (96-DPI) units
fn to_logical(px: i32) -> i32 {
    let s = nwg::scale_factor();
    if s <= 0.0 {
        return px;
    }
    ((px as f64) / s).round() as i32
}

/// Scale a logical (96-DPI) length to physical pixels.
fn dpi_px(logical: i32) -> i32 {
    let s = nwg::scale_factor();
    ((logical as f64) * s).round() as i32
}

fn dpi_px_u(logical: u32) -> u32 {
    dpi_px(logical as i32).max(0) as u32
}

fn build_font(family: &str, size: u32, fallback: &str) -> nwg::Font {
    let mut font = nwg::Font::default();
    // nwg Font::size is logical units; with high-dpi feature it is scaled by DPI/96
    let size = size.clamp(10, 28);
    let family = if family.trim().is_empty() {
        fallback
    } else {
        family.trim()
    };
    let _ = nwg::Font::builder()
        .size(size)
        .family(family)
        .build(&mut font);
    font
}

fn build_ui_font(family: &str, size: u32) -> nwg::Font {
    build_font(family, size, DEFAULT_UI_FONT_FAMILY)
}

fn build_list_font(family: &str, size: u32) -> nwg::Font {
    build_font(family, size, DEFAULT_LIST_FONT_FAMILY)
}

fn apply_font_to_hwnd_tree(root: windows::Win32::Foundation::HWND, hfont: isize) {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{EnumChildWindows, SendMessageW, WM_SETFONT};

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        unsafe {
            let _ = SendMessageW(hwnd, WM_SETFONT, WPARAM(lparam.0 as usize), LPARAM(1));
        }
        BOOL(1)
    }

    unsafe {
        let _ = SendMessageW(root, WM_SETFONT, WPARAM(hfont as usize), LPARAM(1));
        let _ = EnumChildWindows(root, Some(enum_proc), LPARAM(hfont));
    }
}

/// Path used for the last successful load / save (portable-first).
static CONFIG_PATH: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct HistoryConfig {
    #[serde(default)]
    query: Vec<String>,
    #[serde(default)]
    dir: Vec<String>,
    #[serde(default)]
    file_mask: Vec<String>,
    #[serde(default)]
    dir_mask: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct LayoutConfig {
    /// Tree pane width in logical (96-DPI) pixels
    #[serde(default = "default_tree_width")]
    tree_width: u32,
    /// Result list column widths (logical)
    #[serde(default = "default_col_filename_width")]
    col_filename_width: u32,
    #[serde(default = "default_col_line_width")]
    col_line_width: u32,
    #[serde(default = "default_col_content_width")]
    col_content_width: u32,
    /// Main window size (logical)
    #[serde(default = "default_window_width")]
    window_width: u32,
    #[serde(default = "default_window_height")]
    window_height: u32,
    /// Main window position (logical); None = use default
    #[serde(default)]
    window_x: Option<i32>,
    #[serde(default)]
    window_y: Option<i32>,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            tree_width: default_tree_width(),
            col_filename_width: default_col_filename_width(),
            col_line_width: default_col_line_width(),
            col_content_width: default_col_content_width(),
            window_width: default_window_width(),
            window_height: default_window_height(),
            window_x: None,
            window_y: None,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct EditorConfig {
    #[serde(default)]
    path: String,
    #[serde(default)]
    args: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct AppearanceConfig {
    #[serde(default)]
    theme: usize,
    #[serde(default = "default_font_family")]
    font_family: String,
    #[serde(default = "default_font_size")]
    font_size: u32,
    #[serde(default = "default_list_font_family")]
    list_font_family: String,
    #[serde(default = "default_list_font_size")]
    list_font_size: u32,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: 0,
            font_family: default_font_family(),
            font_size: default_font_size(),
            list_font_family: default_list_font_family(),
            list_font_size: default_list_font_size(),
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct AppConfig {
    #[serde(default)]
    history: HistoryConfig,
    #[serde(default)]
    layout: LayoutConfig,
    #[serde(default)]
    editor: EditorConfig,
    #[serde(default)]
    appearance: AppearanceConfig,
}

fn portable_config_path() -> PathBuf {
    PathBuf::from(CONFIG_FILE_NAME)
}

fn appdata_config_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(CONFIG_APP_DIR).join(CONFIG_FILE_NAME)
}

fn set_config_path(path: PathBuf) {
    if let Ok(mut guard) = CONFIG_PATH.lock() {
        *guard = Some(path);
    }
}

fn resolve_save_path() -> PathBuf {
    if let Ok(guard) = CONFIG_PATH.lock() {
        if let Some(ref p) = *guard {
            return p.clone();
        }
    }
    let portable = portable_config_path();
    if portable.is_file() {
        return portable;
    }
    appdata_config_path()
}

fn parse_config_toml(content: &str) -> Option<AppConfig> {
    toml::from_str(content).ok()
}

fn save_config(cfg: &AppConfig) {
    let path = resolve_save_path();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
    if let Ok(content) = toml::to_string_pretty(cfg) {
        if std::fs::write(&path, content).is_ok() {
            set_config_path(path);
        }
    }
}

fn load_config() -> AppConfig {
    // 1. Portable: ./JGrep3.toml
    let portable = portable_config_path();
    if portable.is_file() {
        if let Ok(content) = std::fs::read_to_string(&portable) {
            if let Some(cfg) = parse_config_toml(&content) {
                set_config_path(portable);
                return cfg;
            }
        }
    }

    // 2. AppData: %APPDATA%\JGrep3\JGrep3.toml
    let appdata = appdata_config_path();
    if appdata.is_file() {
        if let Ok(content) = std::fs::read_to_string(&appdata) {
            if let Some(cfg) = parse_config_toml(&content) {
                set_config_path(appdata);
                return cfg;
            }
        }
    }

    // 3. Defaults — first save goes to AppData
    set_config_path(appdata);
    AppConfig::default()
}

/// Newest-first history: move `value` to front, dedupe, cap at HISTORY_MAX.
fn push_history(list: &mut Vec<String>, value: &str) {
    let v = value.trim();
    if v.is_empty() {
        return;
    }
    list.retain(|x| x != v);
    list.insert(0, v.to_string());
    if list.len() > HISTORY_MAX {
        list.truncate(HISTORY_MAX);
    }
}

/// Editable history combobox (CBS_DROPDOWN). nwg's ComboBox is forced to
/// CBS_DROPDOWNLIST and cannot accept free text, so we own the HWND ourselves.
struct HistoryCombo {
    hwnd: isize,
    items: Vec<String>,
}

impl Default for HistoryCombo {
    fn default() -> Self {
        Self {
            hwnd: 0,
            items: Vec::new(),
        }
    }
}

impl Drop for HistoryCombo {
    fn drop(&mut self) {
        self.destroy();
    }
}

impl HistoryCombo {
    fn create(
        parent: windows::Win32::Foundation::HWND,
        font_source: windows::Win32::Foundation::HWND,
    ) -> Self {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, SendMessageW, HMENU, WINDOW_EX_STYLE, WINDOW_STYLE, WS_CHILD,
            WS_TABSTOP, WS_VISIBLE, WS_VSCROLL, WM_SETFONT,
        };
        use windows::core::w;

        const CBS_DROPDOWN: u32 = 0x0002;
        const CBS_AUTOHSCROLL: u32 = 0x0040;
        const WS_BORDER: u32 = 0x0080_0000;
        const WM_GETFONT: u32 = 0x0031;
        const CB_SETITEMHEIGHT: u32 = 0x0153;

        let Ok(hinstance) = (unsafe { GetModuleHandleW(None) }) else {
            return Self::default();
        };

        let style = WS_CHILD.0
            | WS_VISIBLE.0
            | WS_TABSTOP.0
            | WS_VSCROLL.0
            | WS_BORDER
            | CBS_DROPDOWN
            | CBS_AUTOHSCROLL;

        // Create tall enough for dropdown list metrics; layout will resize later.
        let Ok(hwnd) = (unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("COMBOBOX"),
                w!(""),
                WINDOW_STYLE(style),
                0,
                0,
                100,
                225,
                parent,
                HMENU(std::ptr::null_mut()),
                hinstance,
                None,
            )
        }) else {
            return Self::default();
        };

        unsafe {
            // Match nwg global default font (MS UI Gothic 16) from a sibling control
            let font = SendMessageW(font_source, WM_GETFONT, WPARAM(0), LPARAM(0));
            if font.0 != 0 {
                let _ = SendMessageW(hwnd, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
                // Editable combo has a child EDIT — set the same font there too
                if let Some(edit) = {
                    use windows::Win32::Foundation::RECT;
                    use windows::Win32::UI::Controls::{GetComboBoxInfo, COMBOBOXINFO};
                    let mut info = COMBOBOXINFO {
                        cbSize: std::mem::size_of::<COMBOBOXINFO>() as u32,
                        rcItem: RECT::default(),
                        rcButton: RECT::default(),
                        stateButton: Default::default(),
                        hwndCombo: Default::default(),
                        hwndItem: Default::default(),
                        hwndList: Default::default(),
                    };
                    if GetComboBoxInfo(hwnd, &mut info).is_ok() && !info.hwndItem.is_invalid() {
                        Some(info.hwndItem)
                    } else {
                        None
                    }
                } {
                    let _ = SendMessageW(edit, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
                }
            }
            // Selection field height (-1) and list item height (0) for readable text
            let item_h = dpi_px(20) as isize;
            let _ = SendMessageW(hwnd, CB_SETITEMHEIGHT, WPARAM((-1i32) as usize), LPARAM(item_h));
            let _ = SendMessageW(hwnd, CB_SETITEMHEIGHT, WPARAM(0), LPARAM(item_h));
        }

        Self {
            hwnd: hwnd.0 as isize,
            items: Vec::new(),
        }
    }

    fn destroy(&mut self) {
        if self.hwnd == 0 {
            return;
        }
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{DestroyWindow, IsWindow};
        let hwnd = HWND(self.hwnd as _);
        unsafe {
            if IsWindow(hwnd).as_bool() {
                let _ = DestroyWindow(hwnd);
            }
        }
        self.hwnd = 0;
    }

    fn raw(&self) -> Option<windows::Win32::Foundation::HWND> {
        if self.hwnd == 0 {
            None
        } else {
            Some(windows::Win32::Foundation::HWND(self.hwnd as _))
        }
    }

    fn text(&self) -> String {
        use windows::Win32::UI::WindowsAndMessaging::GetWindowTextW;
        let Some(hwnd) = self.raw() else {
            return String::new();
        };
        let mut buf = vec![0u16; 2048];
        let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
        if len <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..len as usize])
    }

    fn set_text(&self, text: &str) {
        use windows::Win32::UI::WindowsAndMessaging::SetWindowTextW;
        use windows::core::HSTRING;
        let Some(hwnd) = self.raw() else { return };
        let _ = unsafe { SetWindowTextW(hwnd, &HSTRING::from(text)) };
    }

    fn set_items(&mut self, items: Vec<String>) {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
        use windows::core::PCWSTR;
        use std::os::windows::ffi::OsStrExt;
        use std::ffi::OsStr;

        const CB_RESETCONTENT: u32 = 0x014B;
        const CB_ADDSTRING: u32 = 0x0143;

        self.items = items;
        let Some(hwnd) = self.raw() else { return };
        unsafe {
            let _ = SendMessageW(hwnd, CB_RESETCONTENT, WPARAM(0), LPARAM(0));
            for item in &self.items {
                let wide: Vec<u16> = OsStr::new(item)
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                let _ = SendMessageW(
                    hwnd,
                    CB_ADDSTRING,
                    WPARAM(0),
                    LPARAM(PCWSTR(wide.as_ptr()).as_ptr() as isize),
                );
            }
        }
    }

    fn apply_history(&mut self, history: &[String], fallback: &str) {
        self.set_items(history.to_vec());
        if let Some(first) = history.first() {
            self.set_text(first);
        } else {
            self.set_text(fallback);
        }
    }

    fn sync_history(&mut self, value: &str) -> Vec<String> {
        let mut list = self.items.clone();
        push_history(&mut list, value);
        let text = value.trim().to_string();
        self.set_items(list.clone());
        self.set_text(&text);
        list
    }

    fn set_position(&self, x: i32, y: i32) {
        use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_NOSIZE, SWP_NOZORDER};
        let Some(hwnd) = self.raw() else { return };
        // Coordinates are logical (96-DPI); convert for HiDPI
        let (x, y) = (dpi_px(x), dpi_px(y));
        let _ = unsafe { SetWindowPos(hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER) };
    }

    fn set_size(&self, w: u32, h: u32) {
        use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_NOMOVE, SWP_NOZORDER};
        let Some(hwnd) = self.raw() else { return };
        // Logical sizes → physical; keep extra height so the dropdown list has room
        let w = dpi_px_u(w) as i32;
        let h = dpi_px(h as i32) + dpi_px(200);
        let _ = unsafe {
            SetWindowPos(hwnd, None, 0, 0, w, h, SWP_NOMOVE | SWP_NOZORDER)
        };
    }

    fn set_enabled(&self, enabled: bool) {
        use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
        let Some(hwnd) = self.raw() else { return };
        unsafe {
            let _ = EnableWindow(hwnd, enabled);
        }
    }

    fn edit_hwnd(&self) -> Option<isize> {
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::Controls::{GetComboBoxInfo, COMBOBOXINFO};
        let hwnd = self.raw()?;
        let mut info = COMBOBOXINFO {
            cbSize: std::mem::size_of::<COMBOBOXINFO>() as u32,
            rcItem: RECT::default(),
            rcButton: RECT::default(),
            stateButton: Default::default(),
            hwndCombo: Default::default(),
            hwndItem: Default::default(),
            hwndList: Default::default(),
        };
        unsafe {
            if GetComboBoxInfo(hwnd, &mut info).is_ok() && !info.hwndItem.is_invalid() {
                Some(info.hwndItem.0 as isize)
            } else {
                None
            }
        }
    }

    fn apply_theme(&self, dark: bool) {
        let Some(hwnd) = self.raw() else { return };
        set_combo_theme(hwnd, dark);
    }

    fn set_item_height(&self, height: i32) {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
        const CB_SETITEMHEIGHT: u32 = 0x0153;
        let Some(hwnd) = self.raw() else { return };
        // height is logical; scale for HiDPI
        let h = dpi_px(height.max(16)) as isize;
        unsafe {
            let _ = SendMessageW(hwnd, CB_SETITEMHEIGHT, WPARAM((-1i32) as usize), LPARAM(h));
            let _ = SendMessageW(hwnd, CB_SETITEMHEIGHT, WPARAM(0), LPARAM(h));
        }
    }
}

/// Expand editor argument template. Placeholders: $f=file, $l=line, $c=column
fn expand_editor_args(template: &str, file: &str, line: usize, column: usize) -> Vec<String> {
    let line_s = line.to_string();
    let col_s = column.to_string();
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;

    for c in template.chars() {
        match c {
            '"' => in_quotes = !in_quotes,
            ' ' | '\t' if !in_quotes => {
                if !cur.is_empty() {
                    args.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        args.push(cur);
    }

    args.into_iter()
        .map(|a| {
            a.replace("$f", file)
                .replace("$F", file)
                .replace("$l", &line_s)
                .replace("$L", &line_s)
                .replace("$c", &col_s)
                .replace("$C", &col_s)
        })
        .collect()
}

// Settings Dialog Window
#[derive(Default, NwgUi)]
pub struct SettingsDialog {
    #[nwg_control(size: (480, 370), title: "設定", flags: "WINDOW")]
    #[nwg_events(OnWindowClose: [SettingsDialog::handle_close])]
    window: nwg::Window,

    #[nwg_resource(title: "エディタ実行ファイルの選択", action: nwg::FileDialogAction::Open, filters: "実行ファイル(*.exe;*.cmd;*.bat)|すべてのファイル(*.*)")]
    exe_dialog: nwg::FileDialog,

    // Appearance group: theme / UI font / list font (each on its own row)
    #[nwg_control(parent: window, position: (15, 12), size: (445, 155), flags: "VISIBLE|BORDER")]
    appearance_frame: nwg::Frame,

    #[nwg_control(parent: appearance_frame, text: "外観", position: (10, 8), size: (200, 20))]
    lbl_appearance: nwg::Label,

    // Row 1: Theme
    #[nwg_control(parent: appearance_frame, text: "テーマ", position: (10, 38), size: (105, 20))]
    lbl_theme: nwg::Label,

    #[nwg_control(parent: appearance_frame, collection: vec!["システム設定".to_string(), "ライトモード".to_string(), "ダークモード".to_string()], position: (120, 35), size: (305, 25))]
    #[nwg_events(OnComboxBoxSelection: [SettingsDialog::handle_theme_change])]
    cb_theme: nwg::ComboBox<String>,

    // Row 2: UI Font + size
    #[nwg_control(parent: appearance_frame, text: "UIフォント", position: (10, 73), size: (105, 20))]
    lbl_font: nwg::Label,

    #[nwg_control(parent: appearance_frame, collection: vec![
        "Meiryo UI".to_string(),
        "Yu Gothic UI".to_string(),
        "MS UI Gothic".to_string(),
        "Segoe UI".to_string(),
        "Meiryo".to_string(),
        "Yu Gothic".to_string(),
        "Consolas".to_string(),
    ], position: (120, 70), size: (225, 25))]
    #[nwg_events(OnComboxBoxSelection: [SettingsDialog::handle_font_change])]
    cb_font: nwg::ComboBox<String>,

    #[nwg_control(parent: appearance_frame, text: "サイズ", position: (355, 73), size: (35, 20))]
    lbl_font_size: nwg::Label,

    #[nwg_control(parent: appearance_frame, collection: vec![
        "10".to_string(), "11".to_string(), "12".to_string(), "13".to_string(),
        "14".to_string(), "15".to_string(), "16".to_string(), "18".to_string(),
        "20".to_string(), "22".to_string(), "24".to_string(),
    ], position: (395, 70), size: (40, 25))]
    #[nwg_events(OnComboxBoxSelection: [SettingsDialog::handle_font_change])]
    cb_font_size: nwg::ComboBox<String>,

    // Row 3: Result list font + size
    #[nwg_control(parent: appearance_frame, text: "結果リストフォント", position: (10, 108), size: (105, 20))]
    lbl_list_font: nwg::Label,

    #[nwg_control(parent: appearance_frame, collection: vec![
        "Cascadia Code".to_string(),
        "Cascadia Mono".to_string(),
        "Consolas".to_string(),
        "Courier New".to_string(),
        "Meiryo UI".to_string(),
        "MS Gothic".to_string(),
        "Segoe UI".to_string(),
    ], position: (120, 105), size: (225, 25))]
    #[nwg_events(OnComboxBoxSelection: [SettingsDialog::handle_font_change])]
    cb_list_font: nwg::ComboBox<String>,

    #[nwg_control(parent: appearance_frame, text: "サイズ", position: (355, 108), size: (35, 20))]
    lbl_list_font_size: nwg::Label,

    #[nwg_control(parent: appearance_frame, collection: vec![
        "10".to_string(), "11".to_string(), "12".to_string(), "13".to_string(),
        "14".to_string(), "15".to_string(), "16".to_string(), "18".to_string(),
        "20".to_string(), "22".to_string(), "24".to_string(),
    ], position: (395, 105), size: (40, 25))]
    #[nwg_events(OnComboxBoxSelection: [SettingsDialog::handle_font_change])]
    cb_list_font_size: nwg::ComboBox<String>,

    // External editor group
    #[nwg_control(parent: window, position: (15, 180), size: (445, 125), flags: "VISIBLE|BORDER")]
    editor_frame: nwg::Frame,

    #[nwg_control(parent: editor_frame, text: "外部エディタ", position: (10, 8), size: (200, 20))]
    lbl_editor: nwg::Label,

    #[nwg_control(parent: editor_frame, text: "パス", position: (10, 35), size: (40, 20))]
    lbl_editor_path: nwg::Label,

    #[nwg_control(parent: editor_frame, text: "", position: (55, 32), size: (295, 25))]
    txt_editor_path: nwg::TextInput,

    #[nwg_control(parent: editor_frame, text: "参照...", position: (360, 32), size: (65, 25))]
    #[nwg_events(OnButtonClick: [SettingsDialog::handle_editor_browse])]
    btn_editor_browse: nwg::Button,

    #[nwg_control(parent: editor_frame, text: "引数", position: (10, 68), size: (40, 20))]
    lbl_editor_args: nwg::Label,

    #[nwg_control(parent: editor_frame, text: "", position: (55, 65), size: (370, 25))]
    txt_editor_args: nwg::TextInput,

    #[nwg_control(parent: editor_frame, text: "$f=ファイル  $l=行番号  $c=列", position: (55, 93), size: (370, 18))]
    lbl_editor_args_help: nwg::Label,

    #[nwg_control(parent: window, text: "閉じる", position: (200, 320), size: (80, 25))]
    #[nwg_events(OnButtonClick: [SettingsDialog::handle_close])]
    btn_close: nwg::Button,

    // Inter-window communication channels
    parent_sender: RefCell<Option<nwg::NoticeSender>>,
}

impl SettingsDialog {
    fn handle_close(&self) {
        self.save_settings();
        self.window.set_visible(false);
    }

    fn handle_theme_change(&self) {
        self.save_settings();
        if let Some(ref sender) = *self.parent_sender.borrow() {
            sender.notice();
        }
    }

    fn handle_font_change(&self) {
        self.save_settings();
        if let Some(ref sender) = *self.parent_sender.borrow() {
            sender.notice();
        }
    }

    fn handle_editor_browse(&self) {
        if self.exe_dialog.run(Some(&self.window)) {
            if let Ok(path) = self.exe_dialog.get_selected_item() {
                self.txt_editor_path.set_text(&path.to_string_lossy());
                self.save_settings();
            }
        }
    }

    fn selected_font_family(&self) -> String {
        self.cb_font
            .selection_string()
            .unwrap_or_else(|| DEFAULT_UI_FONT_FAMILY.to_string())
    }

    fn selected_list_font_family(&self) -> String {
        self.cb_list_font
            .selection_string()
            .unwrap_or_else(|| DEFAULT_LIST_FONT_FAMILY.to_string())
    }

    fn selected_font_size(&self) -> u32 {
        self.cb_font_size
            .selection_string()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_UI_FONT_SIZE)
            .clamp(10, 28)
    }

    fn selected_list_font_size(&self) -> u32 {
        self.cb_list_font_size
            .selection_string()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_LIST_FONT_SIZE)
            .clamp(10, 28)
    }

    fn save_settings(&self) {
        let mut cfg = load_config();
        cfg.editor.path = self.txt_editor_path.text();
        cfg.editor.args = self.txt_editor_args.text();
        cfg.appearance.theme = self.cb_theme.selection().unwrap_or(0);
        cfg.appearance.font_family = self.selected_font_family();
        cfg.appearance.font_size = self.selected_font_size();
        cfg.appearance.list_font_family = self.selected_list_font_family();
        cfg.appearance.list_font_size = self.selected_list_font_size();
        save_config(&cfg);
    }
}



#[derive(Default, NwgUi)]
pub struct JGrepApp {
    // Main Window
    #[nwg_control(size: (1000, 680), position: (150, 100), title: "JGrep3", flags: "MAIN_WINDOW")]
    #[nwg_events(
        OnWindowClose: [JGrepApp::handle_close],
        OnResize: [JGrepApp::handle_resize],
        OnWindowMaximize: [JGrepApp::handle_resize],
        OnMinMaxInfo: [JGrepApp::handle_min_max(SELF, EVT_DATA)]
    )]
    window: nwg::Window,

    // Menu Bar
    // File Menu
    #[nwg_control(parent: window, text: "ファイル(&F)")]
    menu_file_parent: nwg::Menu,

    #[nwg_control(parent: menu_file_parent, text: "終了(&X)")]
    #[nwg_events(OnMenuItemSelected: [JGrepApp::handle_menu_exit])]
    menu_exit: nwg::MenuItem,

    // Options Menu
    #[nwg_control(parent: window, text: "オプション(&O)")]
    menu_options_parent: nwg::Menu,

    #[nwg_control(parent: menu_options_parent, text: "設定(&S)...")]
    #[nwg_events(OnMenuItemSelected: [JGrepApp::handle_setting_open])]
    menu_settings: nwg::MenuItem,

    // Help Menu
    #[nwg_control(parent: window, text: "ヘルプ(&H)")]
    menu_help_parent: nwg::Menu,

    #[nwg_control(parent: menu_help_parent, text: "バージョン情報(&A)")]
    #[nwg_events(OnMenuItemSelected: [JGrepApp::handle_menu_about])]
    menu_about: nwg::MenuItem,

    #[nwg_control]
    #[nwg_events(OnNotice: [JGrepApp::handle_notice])]
    search_notice: nwg::Notice,

    // Notice triggered when settings in SettingsDialog change (or System Menu settings is selected)
    #[nwg_control]
    #[nwg_events(OnNotice: [JGrepApp::handle_setting_notice])]
    setting_notice: nwg::Notice,

    // Notices triggered by Return / Escape key presses
    #[nwg_control]
    #[nwg_events(OnNotice: [JGrepApp::handle_enter_press])]
    enter_notice: nwg::Notice,

    #[nwg_control]
    #[nwg_events(OnNotice: [JGrepApp::handle_esc_press])]
    esc_notice: nwg::Notice,

    // Notice triggered when splitter is dragged
    #[nwg_control]
    #[nwg_events(OnNotice: [JGrepApp::handle_splitter_notice])]
    splitter_notice: nwg::Notice,

    // Persist layout after splitter / column resize
    #[nwg_control]
    #[nwg_events(OnNotice: [JGrepApp::handle_layout_save])]
    layout_save_notice: nwg::Notice,

    // After user resizes a list column divider
    #[nwg_control]
    #[nwg_events(OnNotice: [JGrepApp::handle_column_resize_notice])]
    column_resize_notice: nwg::Notice,

    // Image list for folder tree icons
    #[nwg_resource(size: (16, 16))]
    image_list: nwg::ImageList,

    // File dialog for browsing directory
    #[nwg_resource(title: "検索ディレクトリの選択", action: nwg::FileDialogAction::OpenDirectory)]
    dir_dialog: nwg::FileDialog,

    // Action buttons (placed next to query field)
    #[nwg_control(text: "🔍 検索開始", size: (80, 25))]
    #[nwg_events(OnButtonClick: [JGrepApp::handle_search])]
    btn_search: nwg::Button,

    // Left Pane - Directory Tree
    #[nwg_control]
    #[nwg_events(
        OnTreeItemExpanded: [JGrepApp::handle_tree_expand(SELF, EVT_DATA)],
        OnTreeItemSelectionChanged: [JGrepApp::handle_tree_select(SELF, EVT_DATA)]
    )]
    tree_view: nwg::TreeView,

    // Splitter Bar (Width: 4px, acts as vertical drag handle)
    #[nwg_control(text: "")]
    splitter_bar: nwg::Button,

    // Right Pane - Search Params Group
    #[nwg_control(text: "検索文字列(S):", size: (150, 25))]
    lbl_query: nwg::Label,

    /// Editable CBS_DROPDOWN history combos (created in init_app)
    cb_query: RefCell<HistoryCombo>,

    #[nwg_control(text: "検索ディレクトリ(D):", size: (150, 25))]
    lbl_dir: nwg::Label,

    cb_dir: RefCell<HistoryCombo>,

    #[nwg_control(text: "参照...", size: (60, 25))]
    #[nwg_events(OnButtonClick: [JGrepApp::handle_browse])]
    btn_browse: nwg::Button,

    #[nwg_control(text: "ファイルマスク(M):", size: (150, 25))]
    lbl_file_mask: nwg::Label,

    cb_file_mask: RefCell<HistoryCombo>,

    #[nwg_control(text: "フォルダマスク(K):", size: (150, 25))]
    lbl_dir_mask: nwg::Label,

    cb_dir_mask: RefCell<HistoryCombo>,

    // Checkboxes
    #[nwg_control(text: "サブディレクトリも検索対象(B)", check_state: nwg::CheckBoxState::Checked, size: (220, 25))]
    cb_recursive: nwg::CheckBox,

    #[nwg_control(text: "大文字・小文字を区別する(C)", check_state: nwg::CheckBoxState::Unchecked, size: (220, 25))]
    cb_case_sensitive: nwg::CheckBox,

    #[nwg_control(text: "正規表現を使用しない(N)", check_state: nwg::CheckBoxState::Unchecked, size: (220, 25))]
    cb_regex_disable: nwg::CheckBox,

    #[nwg_control(text: "文字コードを自動判別する(A)", check_state: nwg::CheckBoxState::Checked, size: (220, 25))]
    cb_auto_detect_encoding: nwg::CheckBox,

    // Right Pane - Search Results List (supports double click & column sorting)
    // GRID is drawn manually in NM_CUSTOMDRAW so dark/light grid colors can be controlled
    #[nwg_control(list_style: nwg::ListViewStyle::Detailed, ex_flags: nwg::ListViewExFlags::FULL_ROW_SELECT)]
    #[nwg_events(
        OnListViewDoubleClick: [JGrepApp::handle_list_double_click(SELF, EVT_DATA)],
        OnListViewColumnClick: [JGrepApp::handle_list_column_click(SELF, EVT_DATA)]
    )]
    list_view: nwg::ListView,

    // Status bar
    #[nwg_control]
    status_bar: nwg::StatusBar,

    // Search progress (marquee) shown over the right side of the status bar
    #[nwg_control(size: (160, 16), position: (0, 0), flags: "VISIBLE|MARQUEE")]
    progress_bar: nwg::ProgressBar,

    // Internal state management
    path_map: RefCell<HashMap<isize, PathBuf>>,
    result_file_map: RefCell<HashMap<usize, (Arc<str>, usize, usize)>>,
    search_state: RefCell<Option<SearchState>>,
    is_vscode_available: RefCell<bool>,
    setting_dialog: RefCell<Option<SettingsDialogUi>>,
    _raw_handler: RefCell<Option<nwg::RawEventHandler>>,
    _settings_theme_handler: RefCell<Option<nwg::RawEventHandler>>,
    _frame_theme_handler: RefCell<Option<nwg::RawEventHandler>>,
    _appearance_theme_handler: RefCell<Option<nwg::RawEventHandler>>,
    _status_theme_handler: RefCell<Option<nwg::RawEventHandler>>,
    _listview_handler: RefCell<Option<nwg::RawEventHandler>>,
    _key_handlers: RefCell<Vec<nwg::RawEventHandler>>,
    _splitter_handler: RefCell<Option<nwg::RawEventHandler>>,
    _label_theme_handlers: RefCell<Vec<nwg::RawEventHandler>>,

    // Splitter width state (Rc so raw event handler shares the same cells)
    left_width: Rc<RefCell<i32>>,
    is_dragging_splitter: Rc<RefCell<bool>>,
    splitter_drag_offset: Rc<RefCell<i32>>,

    // Theme state (shared with CTLCOLOR handlers)
    is_dark: Rc<RefCell<bool>>,
    theme_brushes: Rc<ThemeBrushes>,

    /// Keep result-list HFONT alive (separate from UI font)
    list_font: RefCell<Option<nwg::Font>>,

    // Sorting states
    search_results: RefCell<Vec<search::SearchResultItem>>,
    sort_column: RefCell<usize>,
    sort_ascending: RefCell<bool>,

    /// Last list column the user resized (for stretch-after-drag)
    last_resized_col: RefCell<Option<usize>>,
    /// Guard against re-entrant stretch from set_column_width notifications
    stretching_columns: Cell<bool>,
    /// Content-column search highlight
    highlight: Rc<RefCell<Option<HighlightState>>>,
}

impl JGrepApp {
    fn handle_close(&self) {
        // Persist layout before exit
        self.save_layout();
        // Ensure background threads stop
        self.handle_stop();
        nwg::stop_thread_dispatch();
    }

    /// Save window size/pos, tree width, result list column widths into config.
    fn save_layout(&self) {
        let mut cfg = load_config();
        let tw = (*self.left_width.borrow()).max(120) as u32;
        cfg.layout.tree_width = tw;
        cfg.layout.col_filename_width = self.list_col_width_logical(0, DEFAULT_COL_FILENAME_WIDTH);
        cfg.layout.col_line_width = self.list_col_width_logical(1, DEFAULT_COL_LINE_WIDTH);
        cfg.layout.col_content_width = self.list_col_width_logical(2, DEFAULT_COL_CONTENT_WIDTH);

        // window.size/position return logical units when high-dpi is enabled
        let (w, h) = self.window.size();
        let (x, y) = self.window.position();
        cfg.layout.window_width = w.max(700);
        cfg.layout.window_height = h.max(500);
        cfg.layout.window_x = Some(x);
        cfg.layout.window_y = Some(y);

        save_config(&cfg);
    }

    fn handle_layout_save(&self) {
        // Splitter / generic layout save: proportion all columns to 100%
        self.stretch_list_columns();
        self.save_layout();
    }

    fn handle_column_resize_notice(&self) {
        let col = self.last_resized_col.borrow().unwrap_or(0);
        // Keep the resized column; stretch the others to fill 100%
        self.stretch_list_columns_ex(Some(col));
        self.save_layout();
    }

    fn list_col_width_logical(&self, index: usize, fallback: u32) -> u32 {
        // LVM_GETCOLUMNWIDTH returns physical pixels
        if let Some(col) = self.list_view.column(index, 64) {
            to_logical(col.width).max(20) as u32
        } else {
            fallback
        }
    }

    fn apply_list_col_widths(&self, filename: u32, line: u32, content: u32) {
        // set_column_width expects physical pixels (no nwg DPI scaling)
        self.list_view
            .set_column_width(0, dpi_px(filename.max(20) as i32) as isize);
        self.list_view
            .set_column_width(1, dpi_px(line.max(20) as i32) as isize);
        self.list_view
            .set_column_width(2, dpi_px(content.max(20) as i32) as isize);
    }

    fn list_col_width_phys(&self, index: usize, fallback: i32) -> i32 {
        if let Some(col) = self.list_view.column(index, 64) {
            col.width.max(1)
        } else {
            fallback
        }
    }

    fn list_view_client_width_phys(&self) -> i64 {
        use windows::Win32::Foundation::{HWND, RECT};
        use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

        let Some(raw) = self.list_view.handle.hwnd() else {
            return 0;
        };
        let mut rc = RECT::default();
        unsafe {
            if GetClientRect(HWND(raw as _), &mut rc).is_err() {
                return 0;
            }
        }
        (rc.right - rc.left).max(0) as i64
    }

    /// Stretch columns so widths always sum to 100% of list client width.
    /// If `fixed_col` is set, keep that column's current width and distribute the
    /// remainder among the other columns (used after user resizes a divider).
    fn stretch_list_columns_ex(&self, fixed_col: Option<usize>) {
        if self.stretching_columns.get() {
            return;
        }
        let available = (self.list_view_client_width_phys() - 2).max(0);
        if available < 60 {
            return;
        }
        self.stretching_columns.set(true);

        let min = [
            dpi_px(40) as i64,
            dpi_px(30) as i64,
            dpi_px(40) as i64,
        ];
        let mut w = [
            self.list_col_width_phys(0, dpi_px(DEFAULT_COL_FILENAME_WIDTH as i32))
                .max(1) as i64,
            self.list_col_width_phys(1, dpi_px(DEFAULT_COL_LINE_WIDTH as i32))
                .max(1) as i64,
            self.list_col_width_phys(2, dpi_px(DEFAULT_COL_CONTENT_WIDTH as i32))
                .max(1) as i64,
        ];

        if let Some(fixed) = fixed_col.filter(|&i| i < 3) {
            // Keep resized column; stretch the other two to fill remaining space
            let fixed_w = w[fixed].clamp(min[fixed], (available - min[0] - min[1] - min[2] + min[fixed]).max(min[fixed]));
            w[fixed] = fixed_w;
            let rest = (available - fixed_w).max(1);
            let others: Vec<usize> = (0..3).filter(|&i| i != fixed).collect();
            let o0 = others[0];
            let o1 = others[1];
            let o_total = (w[o0] + w[o1]).max(1);
            w[o0] = (rest * w[o0] / o_total).max(min[o0]);
            w[o1] = (rest - w[o0]).max(min[o1]);
            // If mins overflowed, clamp
            if w[o0] + w[o1] > rest {
                w[o0] = (rest / 2).max(1);
                w[o1] = (rest - w[o0]).max(1);
            }
            // Re-apply fixed + exact remainder on last free column
            w[fixed] = fixed_w.min(available - min[o0] - min[o1]);
            let rest2 = available - w[fixed];
            w[o0] = (rest2 * w[o0] / (w[o0] + w[o1]).max(1)).max(min[o0]);
            w[o1] = (rest2 - w[o0]).max(1);
        } else {
            // All three proportional
            let total = (w[0] + w[1] + w[2]).max(1);
            w[0] = (available * w[0] / total).max(min[0]);
            w[1] = (available * w[1] / total).max(min[1]);
            w[2] = (available - w[0] - w[1]).max(1);
            if w[2] < min[2] {
                let need = min[2] - w[2];
                if w[0] >= w[1] && w[0] > min[0] {
                    w[0] -= need.min(w[0] - min[0]);
                } else if w[1] > min[1] {
                    w[1] -= need.min(w[1] - min[1]);
                }
                w[2] = (available - w[0] - w[1]).max(1);
            }
        }

        // Guarantee exact 100%
        let sum = w[0] + w[1] + w[2];
        if sum != available {
            w[2] = (available - w[0] - w[1]).max(1);
        }

        self.list_view.set_column_width(0, w[0] as isize);
        self.list_view.set_column_width(1, w[1] as isize);
        self.list_view.set_column_width(2, w[2] as isize);
        self.stretching_columns.set(false);
    }

    fn stretch_list_columns(&self) {
        self.stretch_list_columns_ex(None);
    }

    fn init_app(&self) {
        // Lock window redraw immediately to prevent initial white flash of child controls
        if let Some(hwnd_raw) = self.window.handle.hwnd() {
            unsafe {
                use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
                use windows::Win32::Foundation::{HWND, WPARAM, LPARAM};
                let _ = SendMessageW(HWND(hwnd_raw as _), 0x000B /* WM_SETREDRAW */, WPARAM(0), LPARAM(0));
            }
        }
        // Load layout from config (logical units)
        let cfg_layout = load_config();
        *self.left_width.borrow_mut() = cfg_layout.layout.tree_width.max(120) as i32;

        // Restore window size / position
        let ww = cfg_layout.layout.window_width.max(700);
        let wh = cfg_layout.layout.window_height.max(500);
        self.window.set_size(ww, wh);
        if let (Some(x), Some(y)) = (cfg_layout.layout.window_x, cfg_layout.layout.window_y) {
            self.window.set_position(x, y);
        }

        // Load embedded application icon (ID: 1) and set it to the Window (WM_SETICON)
        if let Some(hwnd_raw) = self.window.handle.hwnd() {
            use windows::Win32::System::LibraryLoader::GetModuleHandleW;
            use windows::Win32::UI::WindowsAndMessaging::{LoadIconW, SendMessageW, WM_SETICON, ICON_SMALL, ICON_BIG};
            use windows::Win32::Foundation::{HWND, WPARAM, LPARAM};
            use windows::core::PCWSTR;

            unsafe {
                let hinstance = GetModuleHandleW(None).unwrap_or_default();
                // Resource ID 1 is embedded via winres
                let hicon = LoadIconW(hinstance, PCWSTR(1 as *const u16));
                if let Ok(hicon) = hicon {
                    if !hicon.0.is_null() {
                        let hwnd = HWND(hwnd_raw as _);
                        let _ = SendMessageW(hwnd, WM_SETICON, WPARAM(ICON_SMALL as usize), LPARAM(hicon.0 as isize));
                        let _ = SendMessageW(hwnd, WM_SETICON, WPARAM(ICON_BIG as usize), LPARAM(hicon.0 as isize));
                    }
                }
            }
        }

        // Initialize status bar + progress bar
        self.status_bar.set_text(0, "準備完了");
        self.progress_bar.set_range(0..100);
        self.progress_bar.set_pos(0);
        self.progress_bar.set_visible(false);

        // Initialize search result list view headers
        self.list_view.insert_column("ファイル名");
        self.list_view.insert_column("行番号");
        self.list_view.insert_column("内容");
        self.list_view.set_headers_enabled(true);
        self.apply_list_col_widths(
            cfg_layout.layout.col_filename_width,
            cfg_layout.layout.col_line_width,
            cfg_layout.layout.col_content_width,
        );

        // Align Line Number column (index 1) to the right
        if let Some(lv_hwnd_raw) = self.list_view.handle.hwnd() {
            use windows::Win32::UI::Controls::{LVCOLUMNW, LVCF_FMT, LVCFMT_RIGHT, LVM_SETCOLUMNW};
            use windows::Win32::Foundation::{HWND, WPARAM, LPARAM};
            use windows::Win32::UI::WindowsAndMessaging::SendMessageW;

            let lvc = LVCOLUMNW {
                mask: LVCF_FMT,
                fmt: LVCFMT_RIGHT,
                ..Default::default()
            };
            unsafe {
                SendMessageW(
                    HWND(lv_hwnd_raw as _),
                    LVM_SETCOLUMNW,
                    WPARAM(1),
                    LPARAM(&lvc as *const LVCOLUMNW as isize),
                );
            }
        }

        // Force the vertical scrollbar to be always visible (even if empty or disabled)
        // to prevent horizontal layout shift / column stretching artifacts when items are populated.
        if let Some(lv_hwnd) = self.list_view.handle.hwnd() {
            use windows::Win32::Foundation::{HWND, BOOL};
            #[link(name = "user32")]
            unsafe extern "system" {
                fn ShowScrollBar(hWnd: HWND, wBar: i32, bShow: BOOL) -> BOOL;
            }
            unsafe {
                let _ = ShowScrollBar(HWND(lv_hwnd as _), 1 /* SB_VERT */, BOOL(1));
            }
        }

        // Load system icons and insert into ImageList directly
        let icon_indices = [34, 234, 15, 8, 3];
        use windows::Win32::UI::Shell::ExtractIconW;
        use windows::Win32::Foundation::HINSTANCE;
        use windows::core::w;
        use windows::Win32::UI::Controls::{ImageList_ReplaceIcon, HIMAGELIST};

        for &index in &icon_indices {
            let hicon = unsafe {
                ExtractIconW(
                    HINSTANCE(std::ptr::null_mut()),
                    w!("shell32.dll"),
                    index as u32,
                )
            };
            if !hicon.0.is_null() && hicon.0 as isize != 1 {
                unsafe {
                    ImageList_ReplaceIcon(
                        HIMAGELIST(self.image_list.handle as _),
                        -1,
                        hicon,
                    );
                }
            }
        }

        // Bind image list to tree view
        self.tree_view.set_image_list(Some(&self.image_list));

        // Populate tree view Explorer style: Desktop -> Documents / My Computer (Drives) / Desktop Folders
        let desktop_path = get_desktop_path();
        let desktop = self.tree_view.insert_item("デスクトップ", None, nwg::TreeInsert::Root);
        self.tree_view.set_item_image(&desktop, 0, false);
        self.tree_view.set_item_image(&desktop, 0, true);
        self.path_map.borrow_mut().insert(desktop.handle as isize, desktop_path.clone());

        // Under Desktop: Add "My Documents" (Icon index 1)
        let doc_path = get_documents_path();
        self.add_directory_node(Some(&desktop), "マイドキュメント", doc_path, 1);

        // Under Desktop: Add "My Computer" (Icon index 2, virtual item, no path mapping)
        let my_comp = self.tree_view.insert_item("マイ コンピュータ", Some(&desktop), nwg::TreeInsert::Last);
        self.tree_view.set_item_image(&my_comp, 2, false);
        self.tree_view.set_item_image(&my_comp, 2, true);
        
        // Under My Computer: Add Drives (Icon index 3)
        let drives = drives::get_logical_drives();
        for drive in drives {
            self.add_directory_node(Some(&my_comp), &drive, PathBuf::from(&drive), 3);
        }

        // Under Desktop: Add any local folders residing on user's actual Desktop directory (Icon index 4)
        let desktop_subdirs = drives::list_subdirectories(&desktop_path);
        for (name, subpath) in desktop_subdirs {
            self.add_directory_node(Some(&desktop), &name, subpath, 4);
        }

        // Expand root Desktop & My Computer on startup
        self.tree_view.set_expand_state(&desktop, nwg::ExpandState::Expand);
        self.tree_view.set_expand_state(&my_comp, nwg::ExpandState::Expand);

        
        // Check for VS Code availability
        *self.is_vscode_available.borrow_mut() = check_vscode_available();

        // Instantiate settings dialog and load configurations
        let dialog = SettingsDialog::build_ui(Default::default()).expect("Failed to build settings dialog");
        *dialog.parent_sender.borrow_mut() = Some(self.setting_notice.sender());
        
        let cfg = load_config();
        dialog.txt_editor_path.set_text(&cfg.editor.path);
        dialog.txt_editor_args.set_text(&cfg.editor.args);
        dialog.cb_theme.set_selection(Some(cfg.appearance.theme));
        let font_idx = UI_FONT_FAMILIES
            .iter()
            .position(|f| *f == cfg.appearance.font_family.as_str())
            .unwrap_or(0);
        dialog.cb_font.set_selection(Some(font_idx));
        let list_font_idx = LIST_FONT_FAMILIES
            .iter()
            .position(|f| *f == cfg.appearance.list_font_family.as_str())
            .unwrap_or(0);
        dialog.cb_list_font.set_selection(Some(list_font_idx));
        let font_size = if cfg.appearance.font_size == 0 {
            DEFAULT_UI_FONT_SIZE
        } else {
            cfg.appearance.font_size
        };
        let list_font_size = if cfg.appearance.list_font_size == 0 {
            DEFAULT_LIST_FONT_SIZE
        } else {
            cfg.appearance.list_font_size
        };
        dialog.cb_font_size.set_selection(Some(font_size_index(font_size)));
        dialog
            .cb_list_font_size
            .set_selection(Some(font_size_index(list_font_size)));
        let theme_idx = cfg.appearance.theme;
        let font_family = cfg.appearance.font_family.clone();
        let list_font_family = cfg.appearance.list_font_family.clone();

        // Editable history combos (CBS_DROPDOWN — free text + dropdown history)
        {
            use windows::Win32::Foundation::HWND;
            let parent = HWND(self.window.handle.hwnd().unwrap() as _);
            // Use a nwg control that already has the global default font
            let font_src = HWND(self.lbl_query.handle.hwnd().unwrap() as _);
            *self.cb_query.borrow_mut() = HistoryCombo::create(parent, font_src);
            *self.cb_dir.borrow_mut() = HistoryCombo::create(parent, font_src);
            *self.cb_file_mask.borrow_mut() = HistoryCombo::create(parent, font_src);
            *self.cb_dir_mask.borrow_mut() = HistoryCombo::create(parent, font_src);
            self.cb_query.borrow_mut().apply_history(&cfg.history.query, "");
            self.cb_dir.borrow_mut().apply_history(&cfg.history.dir, "");
            self.cb_file_mask.borrow_mut().apply_history(&cfg.history.file_mask, "*.*");
            self.cb_dir_mask
                .borrow_mut()
                .apply_history(&cfg.history.dir_mask, "**;!.git;!node_modules;!target");
        }

        *self.setting_dialog.borrow_mut() = Some(dialog);

        // Bind custom menu item "⚙ 設定(&S)..." (ID: 1001) onto Title Bar System Menu
        use windows::Win32::UI::WindowsAndMessaging::{
            GetSystemMenu, AppendMenuW, GetMenu, GetSubMenu, GetMenuItemCount, SetMenuInfo,
            MF_STRING, MF_SEPARATOR, MENUINFO, MIM_STYLE, MNS_NOCHECK,
        };
        use windows::Win32::Foundation::HWND;

        let hwnd_raw = self.window.handle.hwnd().unwrap();
        let hwnd = HWND(hwnd_raw as _);

        // Shrink left checkmark gutter on popup submenus only (not the menubar itself)
        unsafe {
            let hmenu = GetMenu(hwnd);
            if !hmenu.is_invalid() {
                let mi = MENUINFO {
                    cbSize: std::mem::size_of::<MENUINFO>() as u32,
                    fMask: MIM_STYLE,
                    dwStyle: MNS_NOCHECK,
                    cyMax: 0,
                    hbrBack: windows::Win32::Graphics::Gdi::HBRUSH::default(),
                    dwContextHelpID: 0,
                    dwMenuData: 0,
                };
                let count = GetMenuItemCount(hmenu);
                for i in 0..count {
                    let sub = GetSubMenu(hmenu, i);
                    if !sub.is_invalid() {
                        let _ = SetMenuInfo(sub, &mi);
                    }
                }
            }
        }

        let sys_menu = unsafe { GetSystemMenu(hwnd, false) };
        if !sys_menu.is_invalid() {
            unsafe {
                let _ = AppendMenuW(sys_menu, MF_SEPARATOR, 0, None);
                let _ = AppendMenuW(sys_menu, MF_STRING, 1001, w!("⚙ 設定(&S)..."));
            }
        }

        // Bind raw event handler: system menu settings + theme color messages + listview grid
        let notice_sender = self.setting_notice.sender();
        let is_dark_cell = self.is_dark.clone();
        let brushes = self.theme_brushes.clone();
        let highlight_cell = self.highlight.clone();
        let lv_hwnd_raw = self.list_view.handle.hwnd().map(|h| h as isize).unwrap_or(0);

        let handler = nwg::bind_raw_event_handler(&self.window.handle, 0xFFFF + 1, move |hwnd, msg, wparam, lparam| {
            if msg == 0x0112 /* WM_SYSCOMMAND */ && wparam == 1001 {
                notice_sender.notice();
                return Some(0);
            }

            // WM_NOTIFY — ListView custom draw (highlight + grid)
            if msg == 0x004E && lparam != 0 {
                let hl = highlight_cell.borrow();
                if let Some(ret) = handle_listview_custom_draw(
                    lparam,
                    lv_hwnd_raw,
                    *is_dark_cell.borrow(),
                    hl.as_ref(),
                ) {
                    return Some(ret);
                }
            }

            let dark = *is_dark_cell.borrow();

            if dark {
                use windows::Win32::Foundation::HWND;
                if msg == WM_UAHDRAWMENU {
                    let p_udm = lparam as *const UAHMENU;
                    if !p_udm.is_null() {
                        let udm = unsafe { &*p_udm };
                        use windows::Win32::UI::WindowsAndMessaging::{GetMenuBarInfo, OBJID_MENU, MENUBARINFO};
                        use windows::Win32::Foundation::RECT;
                        use windows::Win32::Graphics::Gdi::{OffsetRect, FillRect};
                        let mut mbi = MENUBARINFO {
                            cbSize: std::mem::size_of::<MENUBARINFO>() as u32,
                            ..Default::default()
                        };
                        unsafe {
                            if GetMenuBarInfo(HWND(hwnd as _), OBJID_MENU, 0, &mut mbi).is_ok() {
                                let mut rc_window = RECT::default();
                                let _ = windows::Win32::UI::WindowsAndMessaging::GetWindowRect(HWND(hwnd as _), &mut rc_window);
                                let mut rc = mbi.rcBar;
                                let _ = OffsetRect(&mut rc, -rc_window.left, -rc_window.top);
                                FillRect(udm.hdc, &rc, brushes.bg(true));
                            }
                        }
                        return Some(1);
                    }
                } else if msg == WM_UAHDRAWMENUITEM {
                    let p_udmi = lparam as *const UAHDRAWMENUITEM;
                    if !p_udmi.is_null() {
                        let udmi = unsafe { &*p_udmi };
                        use windows::Win32::UI::WindowsAndMessaging::{GetMenuItemInfoW, MENUITEMINFOW, MIIM_STRING};
                        use windows::Win32::Graphics::Gdi::{SetTextColor, SetBkMode, DrawTextW, TRANSPARENT, FillRect, CreateSolidBrush, DeleteObject, HGDIOBJ, DT_CENTER, DT_SINGLELINE, DT_VCENTER, DT_HIDEPREFIX};
                        use windows::Win32::Foundation::COLORREF;

                        let mut buf = vec![0u16; 128];
                        let mut mii = MENUITEMINFOW {
                            cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                            fMask: MIIM_STRING,
                            dwTypeData: windows::core::PWSTR(buf.as_mut_ptr()),
                            cch: buf.len() as u32,
                            ..Default::default()
                        };
                        unsafe {
                            let _ = GetMenuItemInfoW(udmi.um.hmenu, udmi.umi.i_position as u32, true, &mut mii);
                            let len = mii.cch as usize;

                            const ODS_HOTLIGHT: u32 = 0x0040;
                            const ODS_SELECTED: u32 = 0x0001;
                            const ODS_GRAYED: u32 = 0x0002;
                            const ODS_DISABLED: u32 = 0x0004;
                            const ODS_NOACCEL: u32 = 0x0100;

                            let state = udmi.dis.itemState.0;

                            let (bg_color, fg_color) = if (state & ODS_SELECTED) != 0 {
                                (0x003A3A3A, CLR_DARK_FG)
                            } else if (state & ODS_HOTLIGHT) != 0 {
                                (0x002D2D2D, CLR_DARK_FG)
                            } else if (state & (ODS_GRAYED | ODS_DISABLED)) != 0 {
                                (CLR_DARK_BG, 0x00808080)
                            } else {
                                (CLR_DARK_BG, CLR_DARK_FG)
                            };

                            let brush = CreateSolidBrush(COLORREF(bg_color));
                            let _ = FillRect(udmi.um.hdc, &udmi.dis.rcItem, brush);
                            let _ = DeleteObject(HGDIOBJ(brush.0));

                            let mut draw_flags = DT_CENTER | DT_SINGLELINE | DT_VCENTER;
                            if (state & ODS_NOACCEL) != 0 {
                                draw_flags |= DT_HIDEPREFIX;
                            }

                            let _ = SetTextColor(udmi.um.hdc, COLORREF(fg_color));
                            let _ = SetBkMode(udmi.um.hdc, TRANSPARENT);
                            let mut rc = udmi.dis.rcItem;
                            let _ = DrawTextW(udmi.um.hdc, &mut buf[..len], &mut rc, draw_flags);
                        }
                        return Some(1);
                    }
                } else if msg == WM_UAHMEASUREMENUITEM {
                    let p_mmi = lparam as *const UAHMEASUREMENUITEM;
                    if !p_mmi.is_null() {
                        use windows::Win32::UI::WindowsAndMessaging::DefWindowProcW;
                        unsafe {
                            let ret = DefWindowProcW(HWND(hwnd as _), msg, WPARAM(wparam), LPARAM(lparam));
                            let p_mmi_mut = lparam as *mut UAHMEASUREMENUITEM;
                            (*p_mmi_mut).mis.itemWidth = (((*p_mmi_mut).mis.itemWidth as u32) * 4 / 3) as u32;
                            return Some(ret.0 as isize);
                        }
                    }
                } else if msg == 0x0085 /* WM_NCPAINT */ || msg == 0x0086 /* WM_NCACTIVATE */ {
                    use windows::Win32::UI::WindowsAndMessaging::DefWindowProcW;
                    let ret = unsafe { DefWindowProcW(HWND(hwnd as _), msg, WPARAM(wparam), LPARAM(lparam)) };
                    uah_draw_menu_nc_bottom_line(HWND(hwnd as _), dark, &brushes);
                    return Some(ret.0 as isize);
                }
            }

            // WM_ERASEBKGND: force custom background paint before controls draw, preventing white flashes
            if msg == 0x0014 {
                use windows::Win32::Graphics::Gdi::FillRect;
                use windows::Win32::Foundation::RECT;
                use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam as _);
                let wh = HWND(hwnd as _);
                let mut rc = RECT::default();
                unsafe {
                    let _ = GetClientRect(wh, &mut rc);
                    let _ = FillRect(hdc, &rc, brushes.bg(dark));
                }
                return Some(1);
            }

            if let Some(ret) = handle_theme_color_msg(hwnd as isize, msg, wparam, &brushes, dark) {
                return Some(ret);
            }
            None
        }).unwrap();
        *self._raw_handler.borrow_mut() = Some(handler);

        // Theme color handler for settings dialog + editor frame (children parented to frame)
        if let Some(ref dialog) = *self.setting_dialog.borrow() {
            let is_dark_cell = self.is_dark.clone();
            let brushes = self.theme_brushes.clone();

            let sh = nwg::bind_raw_event_handler(&dialog.window.handle, 0xFFFF + 4, move |hwnd, msg, wparam, _lparam| {
                let dark = *is_dark_cell.borrow();
                handle_theme_color_msg(hwnd as isize, msg, wparam, &brushes, dark)
            }).unwrap();
            *self._settings_theme_handler.borrow_mut() = Some(sh);

            let is_dark_cell = self.is_dark.clone();
            let brushes = self.theme_brushes.clone();
            let fh = nwg::bind_raw_event_handler(&dialog.editor_frame.handle, 0xFFFF + 5, move |hwnd, msg, wparam, _lparam| {
                let dark = *is_dark_cell.borrow();
                handle_theme_color_msg(hwnd as isize, msg, wparam, &brushes, dark)
            }).unwrap();
            *self._frame_theme_handler.borrow_mut() = Some(fh);

            let is_dark_cell = self.is_dark.clone();
            let brushes = self.theme_brushes.clone();
            let ah = nwg::bind_raw_event_handler(&dialog.appearance_frame.handle, 0xFFFF + 6, move |hwnd, msg, wparam, _lparam| {
                let dark = *is_dark_cell.borrow();
                handle_theme_color_msg(hwnd as isize, msg, wparam, &brushes, dark)
            }).unwrap();
            *self._appearance_theme_handler.borrow_mut() = Some(ah);
        }

        // ListView: header custom draw + column resize → stretch others to 100%
        {
            use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
            use windows::Win32::Foundation::{HWND, WPARAM, LPARAM};
            use windows::Win32::UI::Controls::{
                HDN_ENDTRACKA, HDN_ENDTRACKW, HDN_ITEMCHANGEDW, HDN_TRACKA, HDN_TRACKW,
            };
            let is_dark_cell = self.is_dark.clone();
            let col_resize_sender = self.column_resize_notice.sender();
            let last_col = self.last_resized_col.clone();
            let lv_hwnd_for_header = self.list_view.handle.hwnd().map(|h| h as isize).unwrap_or(0);
            let lvh = nwg::bind_raw_event_handler(&self.list_view.handle, 0xFFFF + 7, move |_hwnd, msg, _wparam, lparam| {
                if msg != 0x004E || lparam == 0 {
                    return None;
                }
                // NMHEADER: NMHDR + iItem + iButton + pitem
                #[repr(C)]
                struct NmHeader {
                    hwnd_from: isize,
                    id_from: usize,
                    code: u32,
                    i_item: i32,
                    i_button: i32,
                    pitem: usize,
                }
                let nm = unsafe { &*(lparam as *const NmHeader) };
                // HDN_* are UINT-encoded negative values
                let is_track_end =
                    nm.code == HDN_ENDTRACKW || nm.code == HDN_ENDTRACKA;
                let is_tracking = nm.code == HDN_TRACKW || nm.code == HDN_TRACKA;
                let is_item_changed = nm.code == HDN_ITEMCHANGEDW || nm.code == ((-320i32) as u32); // HDN_ITEMCHANGEDW or HDN_ITEMCHANGEDA

                if is_track_end || is_tracking || is_item_changed {
                    let col = if nm.i_item >= 0 { nm.i_item as usize } else { 0 };
                    *last_col.borrow_mut() = Some(col.min(2));
                    // Fire notice on both track end and item changed to guarantee 100% layout fit after drag/double-click.
                    if is_track_end || is_item_changed {
                        col_resize_sender.notice();
                    }
                }

                // Resolve current header HWND (may be recreated)
                let header = unsafe {
                    SendMessageW(HWND(lv_hwnd_for_header as _), 0x1000 + 31 /* LVM_GETHEADER */, WPARAM(0), LPARAM(0))
                };
                if header.0 == 0 {
                    return None;
                }
                handle_header_custom_draw(lparam, header.0 as isize, *is_dark_cell.borrow())
            }).unwrap();
            *self._listview_handler.borrow_mut() = Some(lvh);
        }

        // Status bar: paint text/background manually in dark mode
        {
            let is_dark_cell = self.is_dark.clone();
            let brushes = self.theme_brushes.clone();
            let sbh = nwg::bind_raw_event_handler(&self.status_bar.handle, 0xFFFF + 6, move |hwnd, msg, wparam, _lparam| {
                use windows::Win32::Graphics::Gdi::{
                    SetTextColor, SetBkColor, SetBkMode, FillRect, SelectObject, GetStockObject,
                    TextOutW, OPAQUE, DEFAULT_GUI_FONT, HDC, HGDIOBJ,
                };
                use windows::Win32::Foundation::{COLORREF, RECT, HWND, LPARAM, WPARAM};
                use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, SendMessageW};

                let dark = *is_dark_cell.borrow();
                if !dark {
                    return None;
                }

                // WM_ERASEBKGND
                if msg == 0x0014 {
                    let hdc = HDC(wparam as _);
                    let wh = HWND(hwnd as _);
                    let mut rc = RECT::default();
                    unsafe {
                        let _ = GetClientRect(wh, &mut rc);
                        let _ = FillRect(hdc, &rc, brushes.bg(true));
                    }
                    return Some(1);
                }

                // WM_PAINT — draw status text ourselves
                if msg == 0x000F {
                    let wh = HWND(hwnd as _);
                    let mut ps = windows::Win32::Graphics::Gdi::PAINTSTRUCT::default();
                    unsafe {
                        let hdc = windows::Win32::Graphics::Gdi::BeginPaint(wh, &mut ps);
                        let mut rc = RECT::default();
                        let _ = GetClientRect(wh, &mut rc);
                        let _ = FillRect(hdc, &rc, brushes.bg(true));
                        let _ = SetTextColor(hdc, COLORREF(CLR_DARK_FG));
                        let _ = SetBkColor(hdc, COLORREF(CLR_DARK_BG));
                        let _ = SetBkMode(hdc, OPAQUE);
                        let font = GetStockObject(DEFAULT_GUI_FONT);
                        let old = SelectObject(hdc, HGDIOBJ(font.0));

                        // SB_GETTEXTLENGTHW / SB_GETTEXTW for part 0
                        let len_ret = SendMessageW(wh, 0x0400 + 12 /* SB_GETTEXTLENGTHW */, WPARAM(0), LPARAM(0));
                        let len = (len_ret.0 & 0xFFFF) as usize;
                        if len > 0 && len < 1024 {
                            let mut buf = vec![0u16; len + 1];
                            let _ = SendMessageW(
                                wh,
                                0x0400 + 13 /* SB_GETTEXTW */,
                                WPARAM(0),
                                LPARAM(buf.as_mut_ptr() as isize),
                            );
                            let _ = TextOutW(hdc, 4, 3, &buf[..len]);
                        }

                        let _ = SelectObject(hdc, old);
                        let _ = windows::Win32::Graphics::Gdi::EndPaint(wh, &ps);
                    }
                    return Some(0);
                }

                None
            }).unwrap();
            *self._status_theme_handler.borrow_mut() = Some(sbh);
        }

        // Labels: override nwg's internal WM_NCPAINT which paints NC area with
        // COLOR_WINDOW (always light).  We repaint with the correct theme brush.
        {
            let label_handles: &[&nwg::ControlHandle] = &[
                &self.lbl_query.handle,
                &self.lbl_dir.handle,
                &self.lbl_file_mask.handle,
                &self.lbl_dir_mask.handle,
            ];
            let mut lbl_handlers = Vec::new();
            for (i, lbl) in label_handles.iter().enumerate() {
                let is_dark_cell = self.is_dark.clone();
                let brushes = self.theme_brushes.clone();
                let handler_id = 0xFFFF + 10 + i;
                let lh = nwg::bind_raw_event_handler(lbl, handler_id, move |hwnd, msg, _wparam, _lparam| {
                    // WM_NCPAINT
                    if msg == 0x0085 {
                        use windows::Win32::Foundation::{HWND, RECT, POINT};
                        use windows::Win32::Graphics::Gdi::{GetDC, ReleaseDC, FillRect};
                        use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, GetClientRect};
                        let wh = HWND(hwnd as _);
                        let dark = *is_dark_cell.borrow();
                        let brush = brushes.bg(dark);
                        unsafe {
                            let mut window = RECT::default();
                            let mut client = RECT::default();
                            let _ = GetWindowRect(wh, &mut window);
                            let _ = GetClientRect(wh, &mut client);

                            let mut pt1 = POINT { x: window.left, y: window.top };
                            let mut pt2 = POINT { x: window.right, y: window.bottom };
                            let _ = windows::Win32::Graphics::Gdi::ScreenToClient(wh, &mut pt1);
                            let _ = windows::Win32::Graphics::Gdi::ScreenToClient(wh, &mut pt2);

                            let top = RECT {
                                left: 0,
                                top: pt1.y,
                                right: client.right,
                                bottom: client.top,
                            };
                            let bottom = RECT {
                                left: 0,
                                top: client.bottom,
                                right: client.right,
                                bottom: pt2.y,
                            };
                            let hdc = GetDC(wh);
                            let _ = FillRect(hdc, &top, brush);
                            let _ = FillRect(hdc, &bottom, brush);
                            ReleaseDC(wh, hdc);
                        }
                        return Some(0);
                    }
                    None
                }).unwrap();
                lbl_handlers.push(lh);
            }
            *self._label_theme_handlers.borrow_mut() = lbl_handlers;
        }

        // Bind raw event handler specifically to splitter_bar control handle to process drags safely
        let splitter_sender = self.splitter_notice.sender();
        let layout_save_sender = self.layout_save_notice.sender();
        let left_w_cell = self.left_width.clone();
        let is_dragging_cell = self.is_dragging_splitter.clone();
        let drag_offset_cell = self.splitter_drag_offset.clone();

        let sh = nwg::bind_raw_event_handler(&self.splitter_bar.handle, 0xFFFF + 3, move |hwnd, msg, _wparam, _lparam| {
            use windows::Win32::UI::WindowsAndMessaging::{SetCursor, LoadCursorW, IDC_SIZEWE, GetParent, GetClientRect};
            use windows::Win32::UI::Input::KeyboardAndMouse::{SetCapture, ReleaseCapture};
            use windows::Win32::Graphics::Gdi::ScreenToClient;
            use windows::Win32::Foundation::HWND;

            // left_width is stored in logical units
            let left_w_logical = *left_w_cell.borrow();
            let left_w_phys = dpi_px(left_w_logical);
            let is_dragging = *is_dragging_cell.borrow();

            match msg {
                0x0201 => { // WM_LBUTTONDOWN
                    *is_dragging_cell.borrow_mut() = true;
                    let mut pt = windows::Win32::Foundation::POINT::default();
                    unsafe {
                        let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt);
                        let parent_hwnd = GetParent(HWND(hwnd as _)).unwrap();
                        let _ = ScreenToClient(parent_hwnd, &mut pt);
                    }
                    // offset is in physical pixels
                    *drag_offset_cell.borrow_mut() = pt.x - left_w_phys;
                    unsafe { SetCapture(HWND(hwnd as _)); }
                    return Some(0);
                }
                0x0202 => { // WM_LBUTTONUP
                    if is_dragging {
                        *is_dragging_cell.borrow_mut() = false;
                        unsafe { let _ = ReleaseCapture(); }
                        layout_save_sender.notice();
                        return Some(0);
                    }
                }
                0x0200 => { // WM_MOUSEMOVE
                    if is_dragging {
                        let mut pt = windows::Win32::Foundation::POINT::default();
                        unsafe {
                            let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt);
                            let parent_hwnd = GetParent(HWND(hwnd as _)).unwrap();
                            let _ = ScreenToClient(parent_hwnd, &mut pt);
                            
                            let mut rect = windows::Win32::Foundation::RECT::default();
                            let _ = GetClientRect(parent_hwnd, &mut rect);
                            let w = rect.right - rect.left;
                            
                            let offset = *drag_offset_cell.borrow();
                            let new_w_phys = (pt.x - offset).max(dpi_px(150)).min(w - dpi_px(300));
                            *left_w_cell.borrow_mut() = to_logical(new_w_phys).max(120);
                        }
                        splitter_sender.notice();
                        return Some(0);
                    }
                }
                0x0020 => { // WM_SETCURSOR
                    unsafe {
                        let cursor = LoadCursorW(None, IDC_SIZEWE).unwrap();
                        SetCursor(cursor);
                    }
                    return Some(1);
                }
                _ => {}
            }
            None
        }).unwrap();
        *self._splitter_handler.borrow_mut() = Some(sh);

        // Bind Enter/Esc key hooks to major focusable controls (incl. combo edit children)
        let enter_sender = self.enter_notice.sender();
        let esc_sender = self.esc_notice.sender();

        let mut key_handles: Vec<nwg::ControlHandle> = vec![
            self.tree_view.handle,
            self.list_view.handle,
        ];
        for combo in [
            self.cb_query.borrow(),
            self.cb_dir.borrow(),
            self.cb_file_mask.borrow(),
            self.cb_dir_mask.borrow(),
        ] {
            if combo.hwnd != 0 {
                key_handles.push(nwg::ControlHandle::Hwnd(combo.hwnd as _));
            }
            if let Some(edit_hwnd) = combo.edit_hwnd() {
                key_handles.push(nwg::ControlHandle::Hwnd(edit_hwnd as _));
            }
        }

        let mut handlers = Vec::new();
        for (i, ctrl) in key_handles.iter().enumerate() {
            let ent_s = enter_sender.clone();
            let esc_s = esc_sender.clone();
            let kh = nwg::bind_raw_event_handler(ctrl, 0xFFFF + 20 + i, move |_hwnd, msg, wparam, _lparam| {
                if msg == 0x0087 /* WM_GETDLGCODE */ {
                    return Some(4 /* DLGC_WANTALLKEYS */);
                }
                if msg == 0x0100 /* WM_KEYDOWN */ {
                    if wparam == 0x0D /* VK_RETURN */ {
                        ent_s.notice();
                        return Some(0);
                    } else if wparam == 0x1B /* VK_ESCAPE */ {
                        esc_s.notice();
                        return Some(0);
                    }
                }
                None
            }).unwrap();
            handlers.push(kh);
        }
        *self._key_handlers.borrow_mut() = handlers;

        // TreeView indent: wide enough for hierarchy lines to read clearly
        use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
        use windows::Win32::Foundation::{WPARAM, LPARAM};
        let tv_hwnd = HWND(self.tree_view.handle.hwnd().unwrap() as _);
        unsafe {
            SendMessageW(tv_hwnd, 0x1100 + 7 /* TVM_SETINDENT */, WPARAM(18), LPARAM(0));
        }

        // Setup initial controls layout
        self.handle_resize();

        // Apply theme + font after event handlers are bound (so CTLCOLOR works immediately)
        let dark = match theme_idx {
            0 => is_system_dark_mode(),
            1 => false,
            2 => true,
            _ => false,
        };
        self.apply_theme(dark);

        // Apply custom subclass to progress bar to render high-quality custom gradient Marquee
        if let Some(pb_hwnd) = self.progress_bar.handle.hwnd() {
            unsafe {
                use windows::Win32::Foundation::HWND;
                let dark_val = if dark { 1 } else { 0 };
                let _ = SetWindowSubclass(HWND(pb_hwnd as _), progress_bar_subclass_proc, 1001, dark_val);
            }
        }
        self.apply_ui_font(&font_family, font_size);
        self.apply_list_font(&list_font_family, list_font_size);

        // Unlock window redraw and redraw the entire window hierarchy to ensure flat dark color paint.
        if let Some(hwnd_raw) = self.window.handle.hwnd() {
            unsafe {
                use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
                use windows::Win32::Graphics::Gdi::{RedrawWindow, RDW_INVALIDATE, RDW_UPDATENOW, RDW_ALLCHILDREN, RDW_ERASE};
                use windows::Win32::Foundation::{WPARAM, LPARAM, HWND};
                let hwnd = HWND(hwnd_raw as _);
                let _ = SendMessageW(hwnd, 0x000B /* WM_SETREDRAW */, WPARAM(1), LPARAM(0));
                let _ = RedrawWindow(hwnd, None, None, RDW_INVALIDATE | RDW_UPDATENOW | RDW_ALLCHILDREN | RDW_ERASE);
            }
        }

        // Show window only after all layouts, themes, and fonts are fully applied.
        // This eliminates the initial white window flash completely.
        self.window.set_visible(true);
    }

    fn apply_ui_font(&self, family: &str, size: u32) {
        use windows::Win32::Foundation::HWND;

        let font = build_ui_font(family, size);
        let hfont = font.handle as isize;
        nwg::Font::set_global_default(Some(font));

        if let Some(h) = self.window.handle.hwnd() {
            apply_font_to_hwnd_tree(HWND(h as _), hfont);
        }
        if let Some(ref dialog) = *self.setting_dialog.borrow() {
            if let Some(h) = dialog.window.handle.hwnd() {
                apply_font_to_hwnd_tree(HWND(h as _), hfont);
            }
        }

        let item_h = (size as i32 + 4).max(18);
        self.cb_query.borrow().set_item_height(item_h);
        self.cb_dir.borrow().set_item_height(item_h);
        self.cb_file_mask.borrow().set_item_height(item_h);
        self.cb_dir_mask.borrow().set_item_height(item_h);
    }

    fn apply_list_font(&self, family: &str, size: u32) {
        use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_SETFONT};

        let font = build_list_font(family, size);
        let hfont = font.handle as isize;
        // Keep HFONT alive for the lifetime of the app
        *self.list_font.borrow_mut() = Some(font);

        let Some(raw) = self.list_view.handle.hwnd() else { return };
        let lv = HWND(raw as _);
        unsafe {
            let _ = SendMessageW(lv, WM_SETFONT, WPARAM(hfont as usize), LPARAM(1));
            // Header control
            let header = SendMessageW(lv, 0x1000 + 31 /* LVM_GETHEADER */, WPARAM(0), LPARAM(0));
            if header.0 != 0 {
                let _ = SendMessageW(
                    HWND(header.0 as _),
                    WM_SETFONT,
                    WPARAM(hfont as usize),
                    LPARAM(1),
                );
            }
        }
        self.list_view.invalidate();
    }

    fn handle_enter_press(&self) {
        if self.search_state.borrow().is_none() {
            self.handle_search();
        }
    }

    fn handle_esc_press(&self) {
        if self.search_state.borrow().is_some() {
            self.handle_stop();
        }
    }

    fn handle_splitter_notice(&self) {
        self.handle_resize();
    }

    fn handle_list_column_click(&self, data: &nwg::EventData) {
        let (_, col_idx) = data.on_list_view_item_index();
        
        let mut current_col = self.sort_column.borrow_mut();
        let mut ascending = self.sort_ascending.borrow_mut();
        
        if *current_col == col_idx {
            *ascending = !*ascending;
        } else {
            *current_col = col_idx;
            *ascending = true;
        }
        
        self.sort_and_refresh_list(*current_col, *ascending);
    }

    fn update_sort_header_indicators(&self, col_idx: Option<usize>, ascending: bool) {
        const COL_COUNT: usize = 3;
        for i in 0..COL_COUNT {
            let arrow = match col_idx {
                Some(c) if c == i => {
                    Some(if ascending {
                        nwg::ListViewColumnSortArrow::Up
                    } else {
                        nwg::ListViewColumnSortArrow::Down
                    })
                }
                _ => None,
            };
            self.list_view.set_column_sort_arrow(i, arrow);
        }
    }

    fn sort_and_refresh_list(&self, col_idx: usize, ascending: bool) {
        let mut results = self.search_results.borrow_mut();
        if results.is_empty() {
            return;
        }

        // Sort items vector
        results.sort_by(|a, b| {
            let cmp = match col_idx {
                0 => a.file_path.cmp(&b.file_path),
                1 => a.line_number.cmp(&b.line_number),
                2 => a.line_content.cmp(&b.line_content),
                _ => std::cmp::Ordering::Equal,
            };
            if ascending {
                cmp
            } else {
                cmp.reverse()
            }
        });

        // Re-populate ListView
        self.list_view.clear();
        let mut file_map = self.result_file_map.borrow_mut();
        file_map.clear();

        for (idx, item) in results.iter().enumerate() {
            self.list_view.insert_item(nwg::InsertListViewItem {
                index: Some(idx as i32),
                column_index: 0,
                text: Some(item.file_path.to_string()),
                image: None,
            });
            self.list_view.update_item(idx, nwg::InsertListViewItem {
                index: Some(idx as i32),
                column_index: 1,
                text: Some(item.line_number.to_string()),
                image: None,
            });
            self.list_view.update_item(idx, nwg::InsertListViewItem {
                index: Some(idx as i32),
                column_index: 2,
                text: Some(item.line_content.clone()),
                image: None,
            });

            file_map.insert(idx, (Arc::clone(&item.file_path), item.line_number, item.column_number));
        }

        self.update_sort_header_indicators(Some(col_idx), ascending);
    }

    fn add_directory_node(&self, parent: Option<&nwg::TreeItem>, name: &str, path: PathBuf, icon_idx: i32) -> nwg::TreeItem {
        let item = self.tree_view.insert_item(name, parent, nwg::TreeInsert::Sort);
        self.tree_view.set_item_image(&item, icon_idx, false);
        self.tree_view.set_item_image(&item, icon_idx, true);
        
        // Dynamically add dummy node if has subdirectories
        if has_subdirectories(&path) {
            let dummy = self.tree_view.insert_item("__DUMMY__", Some(&item), nwg::TreeInsert::Last);
            self.tree_view.set_item_image(&dummy, 4, false); // Default folder icon
        }

        let hitem = item.handle as isize;
        self.path_map.borrow_mut().insert(hitem, path);
        item
    }

    fn handle_tree_expand(&self, data: &nwg::EventData) {
        let (item, _action) = data.on_tree_item_update();
        let hitem = item.handle as isize;

        let path = {
            let path_map = self.path_map.borrow();
            path_map.get(&hitem).cloned()
        };

        if let Some(path) = path {
            if let Some(first_child) = self.tree_view.first_child(item) {
                if self.tree_view.item_text(&first_child) == Some("__DUMMY__".to_string()) {
                    // Remove dummy and read actual directories
                    self.tree_view.remove_item(&first_child);
                    
                    let subdirs = drives::list_subdirectories(&path);
                    for (name, subpath) in subdirs {
                        self.add_directory_node(Some(item), &name, subpath, 4); // Normal folder icon = 4
                    }
                }
            }
        }
    }

    fn handle_tree_select(&self, data: &nwg::EventData) {
        let (_old, new) = data.on_tree_item_selection_changed();
        let hitem = new.handle as isize;

        let path = {
            let path_map = self.path_map.borrow();
            path_map.get(&hitem).cloned()
        };

        if let Some(path) = path {
            self.cb_dir.borrow().set_text(&path.to_string_lossy());
        }
    }

    fn handle_browse(&self) {
        if self.dir_dialog.run(Some(&self.window)) {
            if let Ok(path) = self.dir_dialog.get_selected_item() {
                self.cb_dir.borrow().set_text(&path.to_string_lossy());
            }
        }
    }

    fn handle_clear_results(&self) {
        self.list_view.clear();
        self.result_file_map.borrow_mut().clear();
        self.search_results.borrow_mut().clear();
        self.update_sort_header_indicators(None, true);
        self.status_bar.set_text(0, "結果をクリアしました。");
        *self.highlight.borrow_mut() = None;
    }

    fn handle_search(&self) {
        // Prevent concurrent searches
        if self.search_state.borrow().is_some() {
            return;
        }

        let search_dir_str = self.cb_dir.borrow().text();
        if search_dir_str.trim().is_empty() {
            nwg::simple_message("エラー", "検索ディレクトリを指定してください。");
            return;
        }
        let search_dir = PathBuf::from(&search_dir_str);
        if !search_dir.exists() || !search_dir.is_dir() {
            nwg::simple_message("エラー", "指定された検索ディレクトリが存在しないか、フォルダではありません。");
            return;
        }

        let query = self.cb_query.borrow().text();
        if query.trim().is_empty() {
            nwg::simple_message("エラー", "検索文字列を指定してください。");
            return;
        }

        let file_mask = self.cb_file_mask.borrow().text();
        let dir_mask = self.cb_dir_mask.borrow().text();

        // Update history (100 max) and persist to config
        let history_query = self.cb_query.borrow_mut().sync_history(&query);
        let history_dir = self.cb_dir.borrow_mut().sync_history(&search_dir_str);
        let history_file_mask = self.cb_file_mask.borrow_mut().sync_history(&file_mask);
        let history_dir_mask = self.cb_dir_mask.borrow_mut().sync_history(&dir_mask);
        {
            let mut cfg = load_config();
            cfg.history.query = history_query;
            cfg.history.dir = history_dir;
            cfg.history.file_mask = history_file_mask;
            cfg.history.dir_mask = history_dir_mask;
            if let Some(ref dialog) = *self.setting_dialog.borrow() {
                cfg.editor.path = dialog.txt_editor_path.text();
                cfg.editor.args = dialog.txt_editor_args.text();
                cfg.appearance.theme = dialog.cb_theme.selection().unwrap_or(0);
            }
            save_config(&cfg);
        }

        let recursive = self.cb_recursive.check_state() == nwg::CheckBoxState::Checked;
        let case_sensitive = self.cb_case_sensitive.check_state() == nwg::CheckBoxState::Checked;
        let is_regex = self.cb_regex_disable.check_state() == nwg::CheckBoxState::Unchecked;
        let auto_detect = self.cb_auto_detect_encoding.check_state() == nwg::CheckBoxState::Checked;

        // Clear previous results and reset sort states
        self.handle_clear_results();
        *self.sort_column.borrow_mut() = 0;
        *self.sort_ascending.borrow_mut() = true;

        // Lock UI controls during search
        self.set_ui_searching_state(true);

        // Spin up background thread
        let cancel_token = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let notice_sender = self.search_notice.sender();

        let cancel_token_clone = cancel_token.clone();
        let query_for_search = query.clone();
        let thread_handle = std::thread::spawn(move || {
            search::run_search(
                search_dir,
                query_for_search,
                file_mask,
                dir_mask,
                recursive,
                case_sensitive,
                is_regex,
                auto_detect,
                cancel_token_clone,
                tx,
                notice_sender,
            );
        });

        *self.search_state.borrow_mut() = Some(SearchState {
            cancel_token,
            receiver: rx,
            thread_handle,
        });

        // Set search highlight keyword
        *self.highlight.borrow_mut() = Some(HighlightState {
            pattern: query.clone(),
            case_sensitive,
            is_regex,
        });

        // Show progress bar & start custom timer animation (30ms interval)
        self.progress_bar.set_visible(true);
        if let Some(pb_hwnd) = self.progress_bar.handle.hwnd() {
            unsafe {
                use windows::Win32::UI::WindowsAndMessaging::SetTimer;
                use windows::Win32::Foundation::HWND;
                let _ = SetTimer(HWND(pb_hwnd as _), 1, 30, None);
            }
        }

        self.status_bar.set_text(0, "検索中...");
    }

    fn handle_stop(&self) {
        let mut state = self.search_state.borrow_mut();
        if let Some(ref mut s) = *state {
            s.cancel_token.store(true, Ordering::Relaxed);
            self.status_bar.set_text(0, "検索を停止しています...");

            // Hide progress bar & stop timer animation
            if let Some(pb_hwnd) = self.progress_bar.handle.hwnd() {
                unsafe {
                    use windows::Win32::UI::WindowsAndMessaging::KillTimer;
                    use windows::Win32::Foundation::HWND;
                    let _ = KillTimer(HWND(pb_hwnd as _), 1);
                }
            }
            self.progress_bar.set_visible(false);
            self.stretch_list_columns();
        }
    }

    fn handle_notice(&self) {
        let mut drained = Vec::new();
        {
            let mut state = self.search_state.borrow_mut();
            if let Some(s) = state.as_mut() {
                while let Ok(status) = s.receiver.try_recv() {
                    drained.push(status);
                }
            }
        }

        for status in drained {
            match status {
                SearchStatus::Match(item) => {
                    // Store memory copy for sorting
                    self.search_results.borrow_mut().push(item.clone());

                    let idx = self.list_view.len();
                    self.list_view.insert_item(nwg::InsertListViewItem {
                        index: Some(idx as i32),
                        column_index: 0,
                        text: Some(item.file_path.to_string()),
                        image: None,
                    });
                    self.list_view.update_item(idx, nwg::InsertListViewItem {
                        index: Some(idx as i32),
                        column_index: 1,
                        text: Some(item.line_number.to_string()),
                        image: None,
                    });
                    self.list_view.update_item(idx, nwg::InsertListViewItem {
                        index: Some(idx as i32),
                        column_index: 2,
                        text: Some(item.line_content),
                        image: None,
                    });

                    self.result_file_map.borrow_mut().insert(idx, (Arc::clone(&item.file_path), item.line_number, item.column_number));
                }
                SearchStatus::Matches(items) => {
                    for item in &items {
                        self.search_results.borrow_mut().push(item.clone());

                        let idx = self.list_view.len();
                        self.list_view.insert_item(nwg::InsertListViewItem {
                            index: Some(idx as i32),
                            column_index: 0,
                            text: Some(item.file_path.to_string()),
                            image: None,
                        });
                        self.list_view.update_item(idx, nwg::InsertListViewItem {
                            index: Some(idx as i32),
                            column_index: 1,
                            text: Some(item.line_number.to_string()),
                            image: None,
                        });
                        self.list_view.update_item(idx, nwg::InsertListViewItem {
                            index: Some(idx as i32),
                            column_index: 2,
                            text: Some(item.line_content.clone()),
                            image: None,
                        });

                        self.result_file_map.borrow_mut().insert(idx, (Arc::clone(&item.file_path), item.line_number, item.column_number));
                    }
                }
                SearchStatus::Progress { scanned_files } => {
                    self.status_bar.set_text(0, &format!("検索中... (走査済みファイル数: {})", scanned_files));
                }
                SearchStatus::Completed { elapsed_ms, total_scanned, match_count } => {
                    let elapsed_sec = elapsed_ms as f64 / 1000.0;
                    self.status_bar.set_text(0, &format!("検索完了。{} 個のマッチ (走査: {}, 時間: {:.3}秒)", match_count, total_scanned, elapsed_sec));
                    self.set_ui_searching_state(false);
                    *self.search_state.borrow_mut() = None;

                    // Hide progress bar & stop timer animation
                    if let Some(pb_hwnd) = self.progress_bar.handle.hwnd() {
                        unsafe {
                            use windows::Win32::UI::WindowsAndMessaging::KillTimer;
                            use windows::Win32::Foundation::HWND;
                            let _ = KillTimer(HWND(pb_hwnd as _), 1);
                        }
                    }
                    self.progress_bar.set_visible(false);
                    self.stretch_list_columns();
                }
                SearchStatus::Error(err) => {
                    nwg::simple_message("検索エラー", &err);
                    self.status_bar.set_text(0, "エラーにより検索を中止しました。");
                    self.set_ui_searching_state(false);
                    *self.search_state.borrow_mut() = None;

                    // Hide progress bar & stop timer animation
                    if let Some(pb_hwnd) = self.progress_bar.handle.hwnd() {
                        unsafe {
                            use windows::Win32::UI::WindowsAndMessaging::KillTimer;
                            use windows::Win32::Foundation::HWND;
                            let _ = KillTimer(HWND(pb_hwnd as _), 1);
                        }
                    }
                    self.progress_bar.set_visible(false);
                    self.stretch_list_columns();
                }
            }
        }
    }

    fn handle_list_double_click(&self, data: &nwg::EventData) {
        let (row_idx, _) = data.on_list_view_item_index();
        let file_map = self.result_file_map.borrow();
        if let Some((path_str, line_num, col_num)) = file_map.get(&row_idx) {
            let (editor_path, editor_args) = {
                let dialog_opt = self.setting_dialog.borrow();
                if let Some(ref dialog) = *dialog_opt {
                    (dialog.txt_editor_path.text(), dialog.txt_editor_args.text())
                } else {
                    (String::new(), String::new())
                }
            };

            if !editor_path.trim().is_empty() {
                use std::os::windows::process::CommandExt;
                let mut cmd = std::process::Command::new("cmd");
                cmd.arg("/c").arg(&editor_path);

                if !editor_args.trim().is_empty() {
                    for arg in expand_editor_args(&editor_args, path_str, *line_num, *col_num) {
                        cmd.arg(arg);
                    }
                } else {
                    // No args template: fall back to known editor conventions
                    let path_lower = editor_path.to_lowercase();
                    if path_lower.contains("code") {
                        cmd.arg("-g").arg(format!("{}:{}:{}", path_str, line_num, col_num));
                    } else if path_lower.contains("sakura") {
                        cmd.arg(format!("-Y={}", line_num)).arg(format!("-X={}", col_num)).arg(path_str.as_ref());
                    } else if path_lower.contains("hidemaru") {
                        cmd.arg(format!("/j{}", line_num)).arg(format!(",{}", col_num)).arg(path_str.as_ref());
                    } else if path_lower.contains("notepad++") {
                        cmd.arg(format!("-n{}", line_num)).arg(format!("-c{}", col_num)).arg(path_str.as_ref());
                    } else {
                        cmd.arg(path_str.as_ref());
                    }
                }
                let _ = cmd.creation_flags(0x08000000).spawn();
            } else {
                // Automatic default fallback editor (VS Code or default handler)
                let is_vscode = *self.is_vscode_available.borrow();
                if is_vscode {
                    use std::os::windows::process::CommandExt;
                    let _ = std::process::Command::new("cmd")
                        .args(&["/c", "code", "-g", &format!("{}:{}:{}", path_str, line_num, col_num)])
                        .creation_flags(0x08000000)
                        .spawn();
                } else {
                    let _ = std::process::Command::new("cmd")
                        .args(&["/c", "start", "", path_str])
                        .spawn();
                }
            }
        }
    }

    fn handle_menu_exit(&self) {
        self.handle_close();
    }

    fn handle_menu_about(&self) {
        nwg::simple_message(
            "バージョン情報",
            "JGrep3 v0.1.0\n\nRustで開発されたネイティブUI検索ソフト\nJGREP2 を参考に作成されました。"
        );
    }

    fn handle_setting_open(&self) {
        if let Some(ref dialog) = *self.setting_dialog.borrow() {
            // Center settings relative to the main window
            let (px, py) = self.window.position();
            let (pw, ph) = self.window.size();
            let (sw, sh) = dialog.window.size();
            
            let sx = px + (pw as i32 - sw as i32) / 2;
            let sy = py + (ph as i32 - sh as i32) / 2;
            
            dialog.window.set_position(sx, sy);
            dialog.window.set_visible(true);
            // Ensure the dialog is foreground / not stuck behind main window
            if let Some(raw) = dialog.window.handle.hwnd() {
                use windows::Win32::UI::WindowsAndMessaging::{SetForegroundWindow, ShowWindow, SW_SHOW};
                use windows::Win32::Foundation::HWND;
                let sh = HWND(raw as _);
                unsafe {
                    let _ = ShowWindow(sh, SW_SHOW);
                    let _ = SetForegroundWindow(sh);
                }
            }
        }
    }

    fn handle_setting_notice(&self) {
        if let Some(ref dialog) = *self.setting_dialog.borrow() {
            if !dialog.window.visible() {
                // Triggered via Title Bar System Menu "⚙ 設定...": Open Settings
                self.handle_setting_open();
            } else {
                // Triggered inside Dialog: Apply theme / font settings
                let theme_idx = dialog.cb_theme.selection().unwrap_or(0);
                let dark = match theme_idx {
                    0 => is_system_dark_mode(), // System setting
                    1 => false,                 // Light Mode
                    2 => true,                  // Dark Mode
                    _ => false,
                };
                self.apply_theme(dark);
                let family = dialog.selected_font_family();
                let size = dialog.selected_font_size();
                let list_family = dialog.selected_list_font_family();
                let list_size = dialog.selected_list_font_size();
                self.apply_ui_font(&family, size);
                self.apply_list_font(&list_family, list_size);
            }
        }
    }

    fn handle_min_max(&self, data: &nwg::EventData) {
        let mm = data.on_min_max();
        mm.set_min_size(700, 500);
    }

    fn apply_theme(&self, dark: bool) {
        use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, SetClassLongPtrW, GCLP_HBRBACKGROUND};
        use windows::Win32::Foundation::{HWND, WPARAM, LPARAM};
        use windows::Win32::Graphics::Gdi::{
            InvalidateRect, RedrawWindow, RDW_ERASE, RDW_FRAME, RDW_INVALIDATE, RDW_ALLCHILDREN,
        };
        *self.is_dark.borrow_mut() = dark;

        // App-wide dark mode preference (menus, immersive UI)
        uxtheme_set_app_mode(dark);

        let hwnd = match self.window.handle.hwnd() {
            Some(h) => HWND(h as _),
            None => return,
        };
        let tv_hwnd = match self.tree_view.handle.hwnd() {
            Some(h) => HWND(h as _),
            None => return,
        };
        let lv_hwnd = match self.list_view.handle.hwnd() {
            Some(h) => HWND(h as _),
            None => return,
        };

        apply_dark_titlebar(hwnd, dark);
        apply_menu_theme(hwnd, &self.theme_brushes, dark);

        // Window class background brush
        let bg_brush = self.theme_brushes.bg(dark);
        unsafe {
            SetClassLongPtrW(hwnd, GCLP_HBRBACKGROUND, bg_brush.0 as isize);
        }

        // Generic explorer theme on common controls (excluding labels)
        let controls: &[&nwg::ControlHandle] = &[
            &self.btn_search.handle,
            &self.btn_browse.handle,
            &self.cb_recursive.handle,
            &self.cb_case_sensitive.handle,
            &self.cb_regex_disable.handle,
            &self.cb_auto_detect_encoding.handle,
            &self.splitter_bar.handle,
        ];
        for h in controls {
            if let Some(raw) = h.hwnd() {
                set_control_theme(HWND(raw as _), dark);
            }
        }

        // Labels: untheme so WM_CTLCOLORSTATIC color applies without ugly borders
        let labels: &[&nwg::ControlHandle] = &[
            &self.lbl_query.handle,
            &self.lbl_dir.handle,
            &self.lbl_file_mask.handle,
            &self.lbl_dir_mask.handle,
        ];
        for h in labels {
            if let Some(raw) = h.hwnd() {
                clear_control_theme(HWND(raw as _));
            }
        }

        // TreeView: untheme so classic + / | hierarchy lines are drawn
        // (Explorer / DarkMode_Explorer uses modern chevron expanders instead)
        clear_control_theme(tv_hwnd);

        // ListView: set theme
        set_control_theme(lv_hwnd, dark);

        // Search history combos
        self.cb_query.borrow().apply_theme(dark);
        self.cb_dir.borrow().apply_theme(dark);
        self.cb_file_mask.borrow().apply_theme(dark);
        self.cb_dir_mask.borrow().apply_theme(dark);

        // Status bar: untheme so custom paint / SB_SETBKCOLOR work
        if let Some(sb) = self.status_bar.handle.hwnd() {
            clear_control_theme(HWND(sb as _));
        }

        // Settings dialog
        if let Some(ref dialog) = *self.setting_dialog.borrow() {
            if let Some(raw) = dialog.window.handle.hwnd() {
                let sh = HWND(raw as _);
                apply_dark_titlebar(sh, dark);
                unsafe {
                    SetClassLongPtrW(sh, GCLP_HBRBACKGROUND, bg_brush.0 as isize);

                    // Extract system gear (cog) icon for Settings dialog title bar
                    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
                    use windows::Win32::UI::WindowsAndMessaging::{LoadIconW, SendMessageW, WM_SETICON, ICON_SMALL, ICON_BIG, HICON};
                    use windows::Win32::UI::Shell::ExtractIconW;
                    use windows::Win32::Foundation::{WPARAM, LPARAM};
                    use windows::core::PCWSTR;
                    use std::os::windows::ffi::OsStrExt;
                    use std::ffi::OsStr;

                    let hinstance = GetModuleHandleW(None).unwrap_or_default();
                    let mut hicon = HICON::default();

                    // Try imageres.dll Resource ID 67 (using negative index -67 for modern settings gear icon)
                    let imageres_path: Vec<u16> = OsStr::new("imageres.dll").encode_wide().chain(Some(0)).collect();
                    let hicon_gear = ExtractIconW(hinstance, PCWSTR(imageres_path.as_ptr()), (-67i32) as u32);
                    if !hicon_gear.0.is_null() && hicon_gear.0 as usize != 1 {
                        hicon = hicon_gear;
                    } else {
                        // Fallback to imageres.dll Resource ID 150 (using negative index -150)
                        let hicon_imageres = ExtractIconW(hinstance, PCWSTR(imageres_path.as_ptr()), (-150i32) as u32);
                        if !hicon_imageres.0.is_null() && hicon_imageres.0 as usize != 1 {
                            hicon = hicon_imageres;
                        } else {
                            // Fallback to shell32.dll Resource ID 22 (using negative index -22)
                            let shell32_path: Vec<u16> = OsStr::new("shell32.dll").encode_wide().chain(Some(0)).collect();
                            let hicon_classic = ExtractIconW(hinstance, PCWSTR(shell32_path.as_ptr()), (-22i32) as u32);
                            if !hicon_classic.0.is_null() && hicon_classic.0 as usize != 1 {
                                hicon = hicon_classic;
                            } else {
                                // Fallback to main app icon
                                if let Ok(hi) = LoadIconW(hinstance, PCWSTR(1 as *const u16)) {
                                    hicon = hi;
                                }
                            }
                        }
                    }

                    if !hicon.0.is_null() {
                        let _ = SendMessageW(sh, WM_SETICON, WPARAM(ICON_SMALL as usize), LPARAM(hicon.0 as isize));
                        let _ = SendMessageW(sh, WM_SETICON, WPARAM(ICON_BIG as usize), LPARAM(hicon.0 as isize));
                    }
                }

                // Buttons
                for h in [
                    &dialog.btn_editor_browse.handle,
                    &dialog.btn_close.handle,
                ] {
                    if let Some(r) = h.hwnd() {
                        set_control_theme(HWND(r as _), dark);
                    }
                }

                // Labels: untheme to avoid border artifacts
                for h in [
                    &dialog.lbl_appearance.handle,
                    &dialog.lbl_theme.handle,
                    &dialog.lbl_font.handle,
                    &dialog.lbl_font_size.handle,
                    &dialog.lbl_list_font.handle,
                    &dialog.lbl_list_font_size.handle,
                    &dialog.lbl_editor.handle,
                    &dialog.lbl_editor_path.handle,
                    &dialog.lbl_editor_args.handle,
                    &dialog.lbl_editor_args_help.handle,
                ] {
                    if let Some(r) = h.hwnd() {
                        clear_control_theme(HWND(r as _));
                    }
                }

                // Frame backgrounds
                for h in [
                    &dialog.appearance_frame.handle,
                    &dialog.editor_frame.handle,
                ] {
                    if let Some(r) = h.hwnd() {
                        clear_control_theme(HWND(r as _));
                        set_control_theme(HWND(r as _), dark);
                    }
                }

                // ComboBox theme
                for h in [
                    &dialog.cb_theme.handle,
                    &dialog.cb_font.handle,
                    &dialog.cb_font_size.handle,
                    &dialog.cb_list_font.handle,
                    &dialog.cb_list_font_size.handle,
                ] {
                    if let Some(r) = h.hwnd() {
                        set_combo_theme(HWND(r as _), dark);
                    }
                }

                // Settings edits
                for eh in [
                    dialog.txt_editor_path.handle.hwnd(),
                    dialog.txt_editor_args.handle.hwnd(),
                ]
                .into_iter()
                .flatten()
                {
                    set_edit_theme(HWND(eh as _), dark);
                }
            }
        }

        let (fg, bg, line) = if dark {
            (CLR_DARK_FG, CLR_DARK_BG, CLR_DARK_LINE)
        } else {
            (CLR_LIGHT_FG, CLR_LIGHT_BG, CLR_LIGHT_LINE)
        };

        unsafe {
            // Ensure classic tree styles: + buttons, lines, lines at root
            {
                use windows::Win32::UI::WindowsAndMessaging::{
                    GetWindowLongW, SetWindowLongW, GWL_STYLE,
                };
                const TVS_HASBUTTONS: i32 = 0x0001;
                const TVS_HASLINES: i32 = 0x0002;
                const TVS_LINESATROOT: i32 = 0x0004;
                let style = GetWindowLongW(tv_hwnd, GWL_STYLE);
                SetWindowLongW(
                    tv_hwnd,
                    GWL_STYLE,
                    style | TVS_HASBUTTONS | TVS_HASLINES | TVS_LINESATROOT,
                );
            }

            // TreeView colors (line color is separate so hierarchy stays readable)
            SendMessageW(tv_hwnd, 0x1100 + 29 /* TVM_SETBKCOLOR */, WPARAM(0), LPARAM(bg as isize));
            SendMessageW(tv_hwnd, 0x1100 + 30 /* TVM_SETTEXTCOLOR */, WPARAM(0), LPARAM(fg as isize));
            SendMessageW(tv_hwnd, 0x1100 + 40 /* TVM_SETLINECOLOR */, WPARAM(0), LPARAM(line as isize));
            // Double-buffer only; keep expandos non-fading so + buttons stay solid
            SendMessageW(tv_hwnd, 0x1100 + 44 /* TVM_SETEXTENDEDSTYLE */, WPARAM(0x0040 | 0x0004), LPARAM(0x0004));

            // ListView colors (BKCOLOR + TEXT + TEXTBK)
            let (r, g, b) = if dark { (32u8, 32, 32) } else { (255, 255, 255) };
            self.list_view.set_background_color(r, g, b);
            SendMessageW(lv_hwnd, 0x1000 + 1 /* LVM_SETBKCOLOR */, WPARAM(0), LPARAM(bg as isize));
            SendMessageW(lv_hwnd, 0x1000 + 36 /* LVM_SETTEXTCOLOR */, WPARAM(0), LPARAM(fg as isize));
            SendMessageW(lv_hwnd, 0x1000 + 38 /* LVM_SETTEXTBKCOLOR */, WPARAM(0), LPARAM(bg as isize));
            // Double-buffer
            SendMessageW(lv_hwnd, 0x1000 + 54 /* LVM_SETEXTENDEDLISTVIEWSTYLE */, WPARAM(0x00010000), LPARAM(0x00010000));

            // Header: set theme instead of untheme to ensure empty spacer area uses modern dark colors
            let header = SendMessageW(lv_hwnd, 0x1000 + 31 /* LVM_GETHEADER */, WPARAM(0), LPARAM(0));
            if header.0 != 0 {
                let hh = HWND(header.0 as _);
                set_control_theme(hh, dark);
                let _ = InvalidateRect(hh, None, true);
            }

            // Status bar background
            if let Some(sb) = self.status_bar.handle.hwnd() {
                let sbh = HWND(sb as _);
                SendMessageW(sbh, 0x2001 /* SB_SETBKCOLOR */, WPARAM(0), LPARAM(bg as isize));
                let _ = InvalidateRect(sbh, None, true);
            }

            // Progress bar theme
            if let Some(pb) = self.progress_bar.handle.hwnd() {
                let pbh = HWND(pb as _);
                clear_control_theme(pbh);
                let dark_val = if dark { 1 } else { 0 };
                let _ = SetWindowSubclass(pbh, progress_bar_subclass_proc, 1001, dark_val);
                let _ = InvalidateRect(pbh, None, true);
            }
        }

        // Force full repaint
        unsafe {
            let _ = InvalidateRect(hwnd, None, true);
            let _ = RedrawWindow(
                hwnd,
                None,
                None,
                RDW_ERASE | RDW_FRAME | RDW_INVALIDATE | RDW_ALLCHILDREN,
            );
            if let Some(ref dialog) = *self.setting_dialog.borrow() {
                if let Some(raw) = dialog.window.handle.hwnd() {
                    let sh = HWND(raw as _);
                    let _ = InvalidateRect(sh, None, true);
                    let _ = RedrawWindow(
                        sh,
                        None,
                        None,
                        RDW_ERASE | RDW_FRAME | RDW_INVALIDATE | RDW_ALLCHILDREN,
                    );
                }
            }
        }

        self.tree_view.invalidate();
        self.list_view.invalidate();
        self.window.invalidate();
    }

    fn set_ui_searching_state(&self, searching: bool) {
        self.btn_search.set_enabled(!searching);
        self.menu_exit.set_enabled(!searching);
        self.menu_settings.set_enabled(!searching);
        self.menu_about.set_enabled(!searching);
        
        self.cb_query.borrow().set_enabled(!searching);
        self.cb_dir.borrow().set_enabled(!searching);
        self.btn_browse.set_enabled(!searching);
        self.cb_file_mask.borrow().set_enabled(!searching);
        self.cb_dir_mask.borrow().set_enabled(!searching);
        
        self.cb_recursive.set_enabled(!searching);
        self.cb_case_sensitive.set_enabled(!searching);
        self.cb_regex_disable.set_enabled(!searching);
        self.cb_auto_detect_encoding.set_enabled(!searching);
    }

    fn handle_resize(&self) {
        let (w_u, h_u) = self.window.size();
        let w = w_u as i32;
        let h = h_u as i32;
        if w < 600 || h < 450 {
            return;
        }

        let top_offset = 5;
        let status_height = 25;
        let main_height = h - top_offset - status_height;

        let left_width = *self.left_width.borrow();

        // Left Pane - TreeView
        self.tree_view.set_position(4, top_offset);
        self.tree_view.set_size(left_width as u32, (main_height - 5) as u32);

        // Splitter Bar
        self.splitter_bar.set_position(left_width + 7, top_offset);
        self.splitter_bar.set_size(4, (main_height - 5) as u32);

        // Right Pane
        let right_x = left_width + 14; // tree_width + margins
        let right_width = w - right_x - 4;

        // Configuration panel (fields layout)
        let config_height = 155;
        self.layout_config_fields(right_x, top_offset, right_width);

        // ListView Result Grid
        let list_y = top_offset + config_height + 5;
        let list_height = main_height - config_height - 10;
        self.list_view.set_position(right_x, list_y);
        self.list_view.set_size(right_width as u32, list_height as u32);
        // Keep column proportions when the list width changes
        self.stretch_list_columns();
        self.layout_progress_bar();

        // Force UI Repaint
        self.tree_view.invalidate();
        self.list_view.invalidate();
        self.window.invalidate();
    }

    fn layout_progress_bar(&self) {
        let (w_u, h_u) = self.window.size();
        let w = w_u as i32;
        let h = h_u as i32;
        let pb_w = 280;
        let pb_h = 16;
        let pb_x = w - pb_w - 20;
        let pb_y = h - 25 + (25 - pb_h) / 2;
        self.progress_bar.set_position(pb_x, pb_y);
        self.progress_bar.set_size(pb_w as u32, pb_h as u32);
    }

    fn layout_config_fields(&self, rx: i32, ry: i32, rw: i32) {
        let lbl_w = 120; // Expanded to prevent wrapping
        let input_x = rx + lbl_w + 5;
        let input_w = rw - lbl_w - 5;

        // Row 1: Search Query & search button
        let btn_w = 100;
        let query_w = input_w - (btn_w + 5);

        self.lbl_query.set_position(rx, ry);
        self.lbl_query.set_size(lbl_w as u32, 25);
        self.cb_query.borrow().set_position(input_x, ry);
        self.cb_query.borrow().set_size(query_w as u32, 25);

        self.btn_search.set_position(input_x + query_w + 5, ry);
        self.btn_search.set_size(btn_w as u32, 25);

        // Row 2: Search Directory (needs browse button)
        let btn_br_w = 65;
        let dir_w = input_w - btn_br_w - 5;
        self.lbl_dir.set_position(rx, ry + 30);
        self.lbl_dir.set_size(lbl_w as u32, 25);
        self.cb_dir.borrow().set_position(input_x, ry + 30);
        self.cb_dir.borrow().set_size(dir_w as u32, 25);
        self.btn_browse.set_position(input_x + dir_w + 5, ry + 30);
        self.btn_browse.set_size(btn_br_w as u32, 25);

        // Row 3: Masks
        let half_w = rw / 2;
        self.lbl_file_mask.set_position(rx, ry + 60);
        self.lbl_file_mask.set_size(lbl_w as u32, 25);
        self.cb_file_mask.borrow().set_position(input_x, ry + 60);
        self.cb_file_mask.borrow().set_size((half_w - lbl_w - 10) as u32, 25);

        let dmask_lbl_x = rx + half_w + 5;
        self.lbl_dir_mask.set_position(dmask_lbl_x, ry + 60);
        self.lbl_dir_mask.set_size(lbl_w as u32, 25);
        self.cb_dir_mask.borrow().set_position(dmask_lbl_x + lbl_w + 5, ry + 60);
        self.cb_dir_mask.borrow().set_size((rw - (half_w + 5 + lbl_w + 5)) as u32, 25);

        // Checkboxes Layout (2x2 Grid)
        let cb_y1 = ry + 95;
        let cb_w = rw / 2 - 10;
        self.cb_recursive.set_position(rx, cb_y1);
        self.cb_recursive.set_size(cb_w as u32, 25);

        self.cb_case_sensitive.set_position(rx + half_w, cb_y1);
        self.cb_case_sensitive.set_size(cb_w as u32, 25);

        let cb_y2 = ry + 125;
        self.cb_regex_disable.set_position(rx, cb_y2);
        self.cb_regex_disable.set_size(cb_w as u32, 25);

        self.cb_auto_detect_encoding.set_position(rx + half_w, cb_y2);
        self.cb_auto_detect_encoding.set_size(cb_w as u32, 25);
    }
}

// Check if VS Code is globally available via CLI path
fn check_vscode_available() -> bool {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("cmd")
        .args(&["/c", "where code"])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .and_then(|mut child| child.wait())
        .map(|status| status.success())
        .unwrap_or(false)
}

// Get system known Desktop path
fn get_desktop_path() -> PathBuf {
    if let Some(user_profile) = std::env::var_os("USERPROFILE") {
        let mut path = PathBuf::from(user_profile);
        path.push("Desktop");
        if path.exists() {
            return path;
        }
    }
    PathBuf::from("C:\\")
}

// Get system known Documents path
fn get_documents_path() -> PathBuf {
    if let Some(user_profile) = std::env::var_os("USERPROFILE") {
        let mut path = PathBuf::from(user_profile);
        path.push("Documents");
        if path.exists() {
            return path;
        }
    }
    PathBuf::from("C:\\")
}

// Check if direct child directories exist
fn has_subdirectories(path: &Path) -> bool {
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.filter_map(|e| e.ok()) {
            if let Ok(file_type) = entry.file_type() {
                if file_type.is_dir() && !entry.path().is_symlink() {
                    if let Some(name) = entry.file_name().to_str() {
                        if !name.starts_with('.') && !name.starts_with('$') {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

fn main() {
    nwg::enable_visual_styles();
    nwg::init().expect("Failed to initialize Native Windows GUI");

    // UI font from config (default: Meiryo UI)
    let cfg = load_config();
    let font_size = if cfg.appearance.font_size == 0 {
        DEFAULT_UI_FONT_SIZE
    } else {
        cfg.appearance.font_size
    };
    let font = build_ui_font(&cfg.appearance.font_family, font_size);
    nwg::Font::set_global_default(Some(font));

    let app = JGrepApp::build_ui(Default::default()).expect("Failed to build GUI application");
    app.init_app();
    nwg::dispatch_thread_events();
}
