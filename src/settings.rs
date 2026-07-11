//! 設定ダイアログ UI。
//!
//! `#[derive(NwgUi)]` による宣言的レイアウト。テーマ・フォント・外部エディタ設定。

use std::cell::RefCell;
use native_windows_gui as nwg;
use native_windows_derive::NwgUi;

use crate::config::*;

pub use settings_dialog_ui::SettingsDialogUi;

// Settings Dialog Window
#[derive(Default, NwgUi)]
pub struct SettingsDialog {
    #[nwg_control(size: (600, 375), position: (300, 300), title: "設定", flags: "WINDOW")]
    #[nwg_events(OnWindowClose: [SettingsDialog::handle_close])]
    pub window: nwg::Window,

    #[nwg_resource(title: "エディタ実行ファイルの選択", action: nwg::FileDialogAction::Open, filters: "実行ファイル(*.exe;*.cmd;*.bat)|すべてのファイル(*.*)")]
    pub exe_dialog: nwg::FileDialog,

    // Appearance group: theme / UI font / list font (each on its own row)
    #[nwg_control(parent: window, size: (570, 155), position: (15, 12), flags: "VISIBLE|BORDER")]
    pub appearance_frame: nwg::Frame,

    #[nwg_control(parent: window, text: "  外観  ", position: (25, 2), size: (60, 20))]
    pub lbl_appearance: nwg::Label,

    // Row 1: Theme
    #[nwg_control(parent: appearance_frame, text: "テーマ", position: (10, 33), size: (70, 20))]
    pub lbl_theme: nwg::Label,

    #[nwg_control(parent: appearance_frame, collection: vec!["システム設定".to_string(), "ライトモード".to_string(), "ダークモード".to_string()], position: (85, 30), size: (465, 25))]
    #[nwg_events(OnComboxBoxSelection: [SettingsDialog::handle_theme_change])]
    pub cb_theme: nwg::ComboBox<String>,

    // Row 2: UI Font + size
    #[nwg_control(parent: appearance_frame, text: "UIフォント", position: (10, 68), size: (70, 20))]
    pub lbl_font: nwg::Label,

    #[nwg_control(parent: appearance_frame, collection: vec![
        "Meiryo UI".to_string(),
        "Yu Gothic UI".to_string(),
        "MS UI Gothic".to_string(),
        "Segoe UI".to_string(),
        "Meiryo".to_string(),
        "Yu Gothic".to_string(),
        "Consolas".to_string(),
    ], position: (85, 65), size: (380, 25))]
    #[nwg_events(OnComboxBoxSelection: [SettingsDialog::handle_font_change])]
    pub cb_font: nwg::ComboBox<String>,

    #[nwg_control(parent: appearance_frame, text: "サイズ", position: (470, 68), size: (35, 20))]
    pub lbl_font_size: nwg::Label,

    #[nwg_control(parent: appearance_frame, collection: vec![
        "10".to_string(), "11".to_string(), "12".to_string(), "13".to_string(),
        "14".to_string(), "15".to_string(), "16".to_string(), "18".to_string(),
        "20".to_string(), "22".to_string(), "24".to_string(),
    ], position: (510, 65), size: (40, 25))]
    #[nwg_events(OnComboxBoxSelection: [SettingsDialog::handle_font_change])]
    pub cb_font_size: nwg::ComboBox<String>,

    // Row 3: Result list font + size
    #[nwg_control(parent: appearance_frame, text: "結果フォント", position: (10, 103), size: (70, 20))]
    pub lbl_list_font: nwg::Label,

    #[nwg_control(parent: appearance_frame, collection: vec![
        "Cascadia Code".to_string(),
        "Cascadia Mono".to_string(),
        "Consolas".to_string(),
        "Courier New".to_string(),
        "Meiryo UI".to_string(),
        "MS Gothic".to_string(),
        "Segoe UI".to_string(),
    ], position: (85, 100), size: (380, 25))]
    #[nwg_events(OnComboxBoxSelection: [SettingsDialog::handle_font_change])]
    pub cb_list_font: nwg::ComboBox<String>,

    #[nwg_control(parent: appearance_frame, text: "サイズ", position: (470, 103), size: (35, 20))]
    pub lbl_list_font_size: nwg::Label,

    #[nwg_control(parent: appearance_frame, collection: vec![
        "10".to_string(), "11".to_string(), "12".to_string(), "13".to_string(),
        "14".to_string(), "15".to_string(), "16".to_string(), "18".to_string(),
        "20".to_string(), "22".to_string(), "24".to_string(),
    ], position: (510, 100), size: (40, 25))]
    #[nwg_events(OnComboxBoxSelection: [SettingsDialog::handle_font_change])]
    pub cb_list_font_size: nwg::ComboBox<String>,

    // External editor group
    #[nwg_control(parent: window, size: (570, 140), position: (15, 180), flags: "VISIBLE|BORDER")]
    pub editor_frame: nwg::Frame,

    #[nwg_control(parent: window, text: "  エディタ  ", position: (25, 170), size: (60, 20))]
    pub lbl_editor: nwg::Label,

    #[nwg_control(parent: editor_frame, text: "パス", position: (10, 33), size: (70, 20))]
    pub lbl_editor_path: nwg::Label,

    #[nwg_control(parent: editor_frame, text: "", position: (85, 30), size: (395, 25))]
    pub txt_editor_path: nwg::TextInput,

    #[nwg_control(parent: editor_frame, text: "参照...", position: (485, 30), size: (65, 25))]
    #[nwg_events(OnButtonClick: [SettingsDialog::handle_editor_browse])]
    pub btn_editor_browse: nwg::Button,

    #[nwg_control(parent: editor_frame, text: "引数", position: (10, 66), size: (70, 20))]
    pub lbl_editor_args: nwg::Label,

    #[nwg_control(parent: editor_frame, text: "", position: (85, 63), size: (465, 25))]
    pub txt_editor_args: nwg::TextInput,

    #[nwg_control(parent: editor_frame, text: "%FILENAME%", position: (85, 93), size: (116, 22))]
    #[nwg_events(OnButtonClick: [SettingsDialog::handle_click_filename])]
    pub btn_help_filename: nwg::Button,

    #[nwg_control(parent: editor_frame, text: "=ファイル名  ", position: (207, 95), size: (60, 18))]
    pub lbl_help_filename_desc: nwg::Label,

    #[nwg_control(parent: editor_frame, text: "%LINE%", position: (273, 93), size: (74, 22))]
    #[nwg_events(OnButtonClick: [SettingsDialog::handle_click_line])]
    pub btn_help_line: nwg::Button,

    #[nwg_control(parent: editor_frame, text: "=行番号  ", position: (353, 95), size: (60, 18))]
    pub lbl_help_line_desc: nwg::Label,

    #[nwg_control(parent: editor_frame, text: "%COL%", position: (419, 93), size: (70, 22))]
    #[nwg_events(OnButtonClick: [SettingsDialog::handle_click_col])]
    pub btn_help_col: nwg::Button,

    #[nwg_control(parent: editor_frame, text: "=列番号", position: (495, 95), size: (55, 18))]
    pub lbl_help_col_desc: nwg::Label,

    #[nwg_control(parent: window, text: "閉じる", position: (260, 335), size: (80, 25))]
    #[nwg_events(OnButtonClick: [SettingsDialog::handle_close])]
    pub btn_close: nwg::Button,

    // Inter-window communication channels
    pub parent_sender: RefCell<Option<nwg::NoticeSender>>,
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

    pub fn selected_font_family(&self) -> String {
        self.cb_font
            .selection_string()
            .unwrap_or_else(|| DEFAULT_UI_FONT_FAMILY.to_string())
    }

    pub fn selected_list_font_family(&self) -> String {
        self.cb_list_font
            .selection_string()
            .unwrap_or_else(|| DEFAULT_LIST_FONT_FAMILY.to_string())
    }

    pub fn selected_font_size(&self) -> u32 {
        self.cb_font_size
            .selection_string()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_UI_FONT_SIZE)
            .clamp(10, 28)
    }

    pub fn selected_list_font_size(&self) -> u32 {
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

    fn insert_placeholder(&self, placeholder: &str) {
        use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
        use windows::Win32::Foundation::{HWND, WPARAM, LPARAM};
        if let Some(hwnd) = self.txt_editor_args.handle.hwnd() {
            self.txt_editor_args.set_focus();
            let wide: Vec<u16> = placeholder.encode_utf16().chain(std::iter::once(0)).collect();
            unsafe {
                SendMessageW(
                    HWND(hwnd as _),
                    0x00C2, // EM_REPLACESEL
                    WPARAM(1),
                    LPARAM(wide.as_ptr() as isize),
                );
            }
            self.save_settings();
        }
    }

    fn handle_click_filename(&self) {
        self.insert_placeholder("%FILENAME%");
    }

    fn handle_click_line(&self) {
        self.insert_placeholder("%LINE%");
    }

    fn handle_click_col(&self) {
        self.insert_placeholder("%COL%");
    }
}
