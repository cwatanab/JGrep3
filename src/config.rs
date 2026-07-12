//! 設定の定義・永続化・既定値。
//!
//! AppConfig は TOML 形式で `JGrep3.toml` に保存される。
//! 読込優先: カレントディレクトリ > `%APPDATA%\JGrep3\`

use native_windows_gui as nwg;
use std::path::PathBuf;

// Helper to query Windows registry for System theme mode
pub fn is_system_dark_mode() -> bool {
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

pub const CONFIG_FILE_NAME: &str = "JGrep3.toml";
pub const CONFIG_APP_DIR: &str = "JGrep3";
pub const HISTORY_MAX: usize = 100;

/// Available UI fonts (Japanese Windows)
pub const UI_FONT_FAMILIES: &[&str] = &[
    "Meiryo UI",
    "Yu Gothic UI",
    "MS UI Gothic",
    "Segoe UI",
    "Meiryo",
    "Yu Gothic",
    "Consolas",
];
pub const DEFAULT_UI_FONT_FAMILY: &str = "Meiryo UI";
pub const DEFAULT_UI_FONT_SIZE: u32 = 16;

/// Result list fonts (prefer monospace for code/search hits)
pub const LIST_FONT_FAMILIES: &[&str] = &[
    "Cascadia Code",
    "Cascadia Mono",
    "Consolas",
    "Courier New",
    "Meiryo UI",
    "MS Gothic",
    "Segoe UI",
];
pub const DEFAULT_LIST_FONT_FAMILY: &str = "Cascadia Code";
pub const DEFAULT_LIST_FONT_SIZE: u32 = 14;

/// Font size choices shown in settings
pub const FONT_SIZES: &[u32] = &[10, 11, 12, 13, 14, 15, 16, 18, 20, 22, 24];

pub fn font_size_index(size: u32) -> usize {
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


pub fn default_font_family() -> String {
    DEFAULT_UI_FONT_FAMILY.to_string()
}

pub fn default_font_size() -> u32 {
    DEFAULT_UI_FONT_SIZE
}

pub fn default_list_font_family() -> String {
    DEFAULT_LIST_FONT_FAMILY.to_string()
}

pub fn default_list_font_size() -> u32 {
    DEFAULT_LIST_FONT_SIZE
}

pub const DEFAULT_TREE_WIDTH: u32 = 240;
pub const DEFAULT_COL_FILENAME_WIDTH: u32 = 350;
pub const DEFAULT_COL_LINE_WIDTH: u32 = 60;
pub const DEFAULT_COL_CONTENT_WIDTH: u32 = 450;
pub const DEFAULT_WINDOW_WIDTH: u32 = 1000;
pub const DEFAULT_WINDOW_HEIGHT: u32 = 680;

pub fn default_tree_width() -> u32 {
    DEFAULT_TREE_WIDTH
}
pub fn default_col_filename_width() -> u32 {
    DEFAULT_COL_FILENAME_WIDTH
}
pub fn default_col_line_width() -> u32 {
    DEFAULT_COL_LINE_WIDTH
}
pub fn default_col_content_width() -> u32 {
    DEFAULT_COL_CONTENT_WIDTH
}
pub fn default_window_width() -> u32 {
    DEFAULT_WINDOW_WIDTH
}
pub fn default_window_height() -> u32 {
    DEFAULT_WINDOW_HEIGHT
}

/// Physical pixels → logical (96-DPI) units
pub fn to_logical(px: i32) -> i32 {
    let s = nwg::scale_factor();
    if s <= 0.0 {
        return px;
    }
    ((px as f64) / s).round() as i32
}

/// Scale a logical (96-DPI) length to physical pixels.
pub fn dpi_px(logical: i32) -> i32 {
    let s = nwg::scale_factor();
    ((logical as f64) * s).round() as i32
}

pub fn dpi_px_u(logical: u32) -> u32 {
    dpi_px(logical as i32).max(0) as u32
}

pub fn build_font(family: &str, size: u32, fallback: &str) -> nwg::Font {
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

pub fn build_ui_font(family: &str, size: u32) -> nwg::Font {
    build_font(family, size, DEFAULT_UI_FONT_FAMILY)
}

pub fn build_list_font(family: &str, size: u32) -> nwg::Font {
    build_font(family, size, DEFAULT_LIST_FONT_FAMILY)
}

pub fn apply_font_to_hwnd_tree(root: windows::Win32::Foundation::HWND, hfont: isize) {
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
pub struct HistoryConfig {
    #[serde(default)]
    pub query: Vec<String>,
    #[serde(default)]
    pub dir: Vec<String>,
    #[serde(default)]
    pub mask: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LayoutConfig {
    /// Tree pane width in logical (96-DPI) pixels
    #[serde(default = "default_tree_width")]
    pub tree_width: u32,
    /// Result list column widths (logical)
    #[serde(default = "default_col_filename_width")]
    pub col_filename_width: u32,
    #[serde(default = "default_col_line_width")]
    pub col_line_width: u32,
    #[serde(default = "default_col_content_width")]
    pub col_content_width: u32,
    /// Main window size (logical)
    #[serde(default = "default_window_width")]
    pub window_width: u32,
    #[serde(default = "default_window_height")]
    pub window_height: u32,
    /// Main window position (logical); None = use default
    #[serde(default)]
    pub window_x: Option<i32>,
    #[serde(default)]
    pub window_y: Option<i32>,
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

fn default_editor_path() -> String {
    "code.exe".to_string()
}

fn default_editor_args() -> String {
    "-g %FILENAME%:%LINE%:%COL%".to_string()
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EditorConfig {
    #[serde(default = "default_editor_path")]
    pub path: String,
    #[serde(default = "default_editor_args")]
    pub args: String,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            path: default_editor_path(),
            args: default_editor_args(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AppearanceConfig {
    #[serde(default)]
    pub theme: usize,
    #[serde(default = "default_font_family")]
    pub font_family: String,
    #[serde(default = "default_font_size")]
    pub font_size: u32,
    #[serde(default = "default_list_font_family")]
    pub list_font_family: String,
    #[serde(default = "default_list_font_size")]
    pub list_font_size: u32,
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
pub struct AppConfig {
    #[serde(default)]
    pub history: HistoryConfig,
    #[serde(default)]
    pub layout: LayoutConfig,
    #[serde(default)]
    pub editor: EditorConfig,
    #[serde(default)]
    pub appearance: AppearanceConfig,
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

pub fn save_config(cfg: &AppConfig) {
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

pub fn load_config() -> AppConfig {
    load_config_raw()
}

fn load_config_raw() -> AppConfig {
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
pub fn push_history(list: &mut Vec<String>, value: &str) {
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
