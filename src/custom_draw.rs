//! カスタム描画（NM_CUSTOMDRAW / WM_PAINT ハンドラ）。
//!
//! リストビューのグリッド線・ヘッダー背景・検索ハイライト、
//! およびプログレスバーのアニメーション描画を担当。

use crate::theme::*;
use regex::RegexBuilder;

pub static PROGRESS_PHASE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

#[link(name = "comctl32")]
unsafe extern "system" {
    pub fn SetWindowSubclass(
        hWnd: windows::Win32::Foundation::HWND,
        pfnSubclass: unsafe extern "system" fn(
            windows::Win32::Foundation::HWND,
            u32,
            windows::Win32::Foundation::WPARAM,
            windows::Win32::Foundation::LPARAM,
            usize,
            usize,
        ) -> windows::Win32::Foundation::LRESULT,
        uIdSubclass: usize,
        dwRefData: usize,
    ) -> windows::Win32::Foundation::BOOL;

    fn DefSubclassProc(
        hWnd: windows::Win32::Foundation::HWND,
        uMsg: u32,
        wParam: windows::Win32::Foundation::WPARAM,
        lParam: windows::Win32::Foundation::LPARAM,
    ) -> windows::Win32::Foundation::LRESULT;
}

pub unsafe extern "system" fn progress_bar_subclass_proc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
    _id: usize,
    ref_data: usize,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::{LRESULT, RECT, COLORREF, BOOL};
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, EndPaint, CreateCompatibleDC, CreateCompatibleBitmap, SelectObject, DeleteDC,
        DeleteObject, FillRect, CreateSolidBrush, PAINTSTRUCT, HGDIOBJ, InvalidateRect,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
    use std::sync::atomic::Ordering;

    const WM_PAINT: u32 = 0x000F;
    const WM_TIMER: u32 = 0x0113;
    const WM_ERASEBKGND: u32 = 0x0014;

    let dark = ref_data != 0;

    unsafe {
        match msg {
            WM_ERASEBKGND => LRESULT(1),
            WM_TIMER => {
                let cur = PROGRESS_PHASE.load(Ordering::Relaxed);
                PROGRESS_PHASE.store((cur + 2) % 360, Ordering::Relaxed);
                let _ = InvalidateRect(hwnd, None, BOOL(0));
                LRESULT(0)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                if !hdc.0.is_null() {
                    let mut rc = RECT::default();
                    let _ = GetClientRect(hwnd, &mut rc);
                    let w = rc.right - rc.left;
                    let h = rc.bottom - rc.top;

                    let mem_dc = CreateCompatibleDC(hdc);
                    let mem_bmp = CreateCompatibleBitmap(hdc, w, h);
                    let old_bmp = SelectObject(mem_dc, HGDIOBJ(mem_bmp.0));

                    // Draw sleek flat background
                    let bg_color = if dark { 0x001C1C1C } else { 0x00E0E0E0 };
                    let bg_brush = CreateSolidBrush(COLORREF(bg_color));
                    let _ = FillRect(mem_dc, &rc, bg_brush);
                    let _ = DeleteObject(HGDIOBJ(bg_brush.0));

                    // Neon gradient flow bar
                    let phase = PROGRESS_PHASE.load(Ordering::Relaxed) as f32 / 360.0;
                    let bar_width = (w as f32 * 0.35).max(80.0) as i32;
                    let total_travel = w + bar_width;
                    let start_x = (total_travel as f32 * phase) as i32 - bar_width;

                    for dx in 0..bar_width {
                        let t = dx as f32 / bar_width as f32;
                        let (r, g, b) = if dark {
                            // Cyan (0, 240, 255) -> Purple (180, 0, 255)
                            let r = (0.0 * (1.0 - t) + 180.0 * t) as u32;
                            let g = (240.0 * (1.0 - t) + 0.0 * t) as u32;
                            let b = (255.0 * (1.0 - t) + 255.0 * t) as u32;
                            (r, g, b)
                        } else {
                            // Soft Blue (0, 150, 240) -> Soft Purple (160, 100, 240)
                            let r = (0.0 * (1.0 - t) + 160.0 * t) as u32;
                            let g = (150.0 * (1.0 - t) + 100.0 * t) as u32;
                            let b = (240.0 * (1.0 - t) + 240.0 * t) as u32;
                            (r, g, b)
                        };
                        let color = (b << 16) | (g << 8) | r;
                        let x = start_x + dx;
                        if x >= 0 && x < w {
                            let col_rect = RECT {
                                left: x,
                                top: 0,
                                right: x + 1,
                                bottom: h,
                            };
                            let col_brush = CreateSolidBrush(COLORREF(color));
                            let _ = FillRect(mem_dc, &col_rect, col_brush);
                            let _ = DeleteObject(HGDIOBJ(col_brush.0));
                        }
                    }

                    // Smooth border outline
                    let border_color = if dark { 0x00282828 } else { 0x00CCCCCC };
                    let border_brush = CreateSolidBrush(COLORREF(border_color));
                    let _ = windows::Win32::Graphics::Gdi::FrameRect(mem_dc, &rc, border_brush);
                    let _ = DeleteObject(HGDIOBJ(border_brush.0));

                    let _ = windows::Win32::Graphics::Gdi::BitBlt(hdc, 0, 0, w, h, mem_dc, 0, 0, windows::Win32::Graphics::Gdi::SRCCOPY);

                    let _ = SelectObject(mem_dc, old_bmp);
                    let _ = DeleteObject(HGDIOBJ(mem_bmp.0));
                    let _ = DeleteDC(mem_dc);
                }
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            _ => DefSubclassProc(hwnd, msg, wparam, lparam),
        }
    }
}

/// Header control NM_CUSTOMDRAW (parent is ListView): force correct text/bg colors.
pub fn handle_header_custom_draw(lparam: isize, header_hwnd: isize, dark: bool) -> Option<isize> {
    use windows::Win32::Foundation::{COLORREF, RECT};
    use windows::Win32::Graphics::Gdi::{SetTextColor, SetBkMode, FillRect, HDC, TRANSPARENT};

    #[repr(C)]
    struct NmHdr {
        hwnd_from: isize,
        id_from: usize,
        code: u32,
    }
    #[repr(C)]
    struct NmCustomDraw {
        hdr: NmHdr,
        dw_draw_stage: u32,
        hdc: isize,
        rc: RECT,
        dw_item_spec: usize,
        u_item_state: u32,
        l_iteml_param: isize,
    }

    const NM_CUSTOMDRAW: u32 = (-12i32) as u32;
    const CDDS_PREPAINT: u32 = 0x00000001;
    const CDDS_ITEMPREPAINT: u32 = 0x00010001;
    const CDRF_NOTIFYITEMDRAW: isize = 0x00000020;
    const CDRF_DODEFAULT: isize = 0x00000000;
    const CDRF_SKIPDEFAULT: isize = 0x00000004;
    const CDRF_NOTIFYPOSTPAINT: isize = 0x00000010;

    let nm = lparam as *const NmCustomDraw;
    if nm.is_null() {
        return None;
    }
    let cd = unsafe { &*nm };
    if cd.hdr.hwnd_from != header_hwnd || cd.hdr.code != NM_CUSTOMDRAW {
        return None;
    }

    let (fg, bg) = if dark {
        (CLR_DARK_FG, CLR_DARK_BG) // Match body background for seamless visual flow
    } else {
        (CLR_LIGHT_FG, CLR_LIGHT_BG)
    };

    match cd.dw_draw_stage {
        CDDS_PREPAINT => {
            // Fill header background (empty space beyond columns)
            let hdc = HDC(cd.hdc as _);
            unsafe {
                let brush = windows::Win32::Graphics::Gdi::CreateSolidBrush(COLORREF(bg));
                let _ = FillRect(hdc, &cd.rc, brush);
                let _ = windows::Win32::Graphics::Gdi::DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(brush.0));
            }
            Some(CDRF_NOTIFYITEMDRAW | CDRF_NOTIFYPOSTPAINT)
        }
        s if s == 0x00000002 => { // CDDS_POSTPAINT
            let hdc = HDC(cd.hdc as _);
            unsafe {
                use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
                use windows::Win32::Foundation::{WPARAM, LPARAM, HWND, RECT};
                let mut last_item_rc = RECT::default();
                // Send HDM_GETITEMRECT for index 2 (last column) to get its right-most position
                let res = SendMessageW(
                    HWND(header_hwnd as _),
                    0x1200 + 7, // HDM_GETITEMRECT
                    WPARAM(2),
                    LPARAM(&mut last_item_rc as *mut _ as isize),
                );
                if res.0 != 0 {
                    let right_limit = last_item_rc.right;
                    if right_limit < cd.rc.right {
                        let spacer_rc = RECT {
                            left: right_limit,
                            top: cd.rc.top,
                            right: cd.rc.right,
                            bottom: cd.rc.bottom,
                        };
                        let brush = windows::Win32::Graphics::Gdi::CreateSolidBrush(COLORREF(bg));
                        let _ = FillRect(hdc, &spacer_rc, brush);
                        let _ = windows::Win32::Graphics::Gdi::DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(brush.0));
                    }
                }
            }
            Some(CDRF_DODEFAULT)
        }
        CDDS_ITEMPREPAINT => {
            let hdc = HDC(cd.hdc as _);
            unsafe {
                // Fill item background
                let brush = windows::Win32::Graphics::Gdi::CreateSolidBrush(COLORREF(bg));
                let _ = FillRect(hdc, &cd.rc, brush);
                let _ = windows::Win32::Graphics::Gdi::DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(brush.0));

                // If this is a placeholder / empty spacer item beyond the 3 main columns,
                // do not draw any borders or retrieve text. Just skip default to prevent extra line artifacts.
                if cd.dw_item_spec > 2 {
                    return Some(CDRF_DODEFAULT);
                }

                // Draw border line (right & bottom separators)
                let border_color = if dark { 0x00404040 } else { 0x00D0D0D0 };
                let pen = windows::Win32::Graphics::Gdi::CreatePen(windows::Win32::Graphics::Gdi::PS_SOLID, 1, COLORREF(border_color));
                let old_pen = windows::Win32::Graphics::Gdi::SelectObject(hdc, windows::Win32::Graphics::Gdi::HGDIOBJ(pen.0));

                if cd.dw_item_spec < 2 {
                    let _ = windows::Win32::Graphics::Gdi::MoveToEx(hdc, cd.rc.right - 1, cd.rc.top, None);
                    let _ = windows::Win32::Graphics::Gdi::LineTo(hdc, cd.rc.right - 1, cd.rc.bottom);
                }

                let _ = windows::Win32::Graphics::Gdi::MoveToEx(hdc, cd.rc.left, cd.rc.bottom - 1, None);
                let _ = windows::Win32::Graphics::Gdi::LineTo(hdc, cd.rc.right, cd.rc.bottom - 1);

                let _ = windows::Win32::Graphics::Gdi::SelectObject(hdc, old_pen);
                let _ = windows::Win32::Graphics::Gdi::DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(pen.0));

                // Retrieve column text
                let mut buf = vec![0u16; 128];
                let mut hdi = windows::Win32::UI::Controls::HDITEMW {
                    mask: windows::Win32::UI::Controls::HDI_TEXT,
                    pszText: windows::core::PWSTR(buf.as_mut_ptr()),
                    cchTextMax: buf.len() as i32,
                    ..Default::default()
                };

                use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
                use windows::Win32::Foundation::{HWND, WPARAM, LPARAM};
                let has_item = SendMessageW(
                    HWND(header_hwnd as _),
                    0x1200 + 11, // HDM_GETITEMW = 0x120B
                    WPARAM(cd.dw_item_spec),
                    LPARAM(&mut hdi as *mut windows::Win32::UI::Controls::HDITEMW as isize),
                );

                if has_item.0 != 0 {
                    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                    let text = &mut buf[..len];

                    let _ = SetTextColor(hdc, COLORREF(fg));
                    let _ = SetBkMode(hdc, TRANSPARENT);

                    // Add padding to prevent text touching borders
                    let mut text_rc = cd.rc;
                    text_rc.left += 8;
                    text_rc.right -= 8;

                    let _ = windows::Win32::Graphics::Gdi::DrawTextW(
                        hdc,
                        text,
                        &mut text_rc,
                        windows::Win32::Graphics::Gdi::DT_LEFT | windows::Win32::Graphics::Gdi::DT_VCENTER | windows::Win32::Graphics::Gdi::DT_SINGLELINE,
                    );
                }
            }
            Some(CDRF_SKIPDEFAULT)
        }
        _ => Some(CDRF_DODEFAULT),
    }
}

/// Highlight settings for result list content column
#[derive(Clone)]
pub struct HighlightState {
    pub pattern: String,
    pub case_sensitive: bool,
    pub is_regex: bool,
}

pub fn find_highlight_ranges(text: &str, hl: &HighlightState) -> Vec<(usize, usize)> {
    if hl.pattern.is_empty() {
        return Vec::new();
    }
    let re = if hl.is_regex {
        RegexBuilder::new(&hl.pattern)
            .case_insensitive(!hl.case_sensitive)
            .build()
            .ok()
    } else {
        RegexBuilder::new(&regex::escape(&hl.pattern))
            .case_insensitive(!hl.case_sensitive)
            .build()
            .ok()
    };
    match re {
        Some(re) => re.find_iter(text).map(|m| (m.start(), m.end())).collect(),
        None => Vec::new(),
    }
}

pub fn draw_postpaint_highlights(
    hdc: windows::Win32::Graphics::Gdi::HDC,
    text: &str,
    ranges: &[(usize, usize)],
    rc: &windows::Win32::Foundation::RECT,
    dark: bool,
    _selected: bool,
) {
    use windows::Win32::Foundation::{COLORREF, RECT};
    use windows::Win32::Graphics::Gdi::{
        ExtTextOutW, GetTextExtentPoint32W, SetBkColor, SetBkMode, SetTextColor, OPAQUE,
    };
    use windows::core::PCWSTR;
    use std::os::windows::ffi::OsStrExt;
    use std::ffi::OsStr;

    // Highlight colors (BGR representation)
    let (hl_fg, hl_bg) = if dark {
        (0x00000000, 0x0000D7FF) // black on gold
    } else {
        (0x00000000, 0x0080FFFF) // black on bright yellow
    };

    unsafe {
        let _ = SetBkMode(hdc, OPAQUE);
        let _ = SetTextColor(hdc, COLORREF(hl_fg));
        let _ = SetBkColor(hdc, COLORREF(hl_bg));


        
        let start_x = rc.left + 4; // Native ListView text margin
        let y = rc.top + 2;

        for &(s, e) in ranges {
            if s >= text.len() || e > text.len() || s >= e {
                continue;
            }

            let prefix = &text[..s];
            let wide_prefix: Vec<u16> = OsStr::new(prefix).encode_wide().collect();
            let mut prefix_size = windows::Win32::Foundation::SIZE::default();
            let _ = GetTextExtentPoint32W(hdc, &wide_prefix, &mut prefix_size);
            
            let match_text = &text[s..e];
            let wide_match: Vec<u16> = OsStr::new(match_text).encode_wide().collect();
            let mut match_size = windows::Win32::Foundation::SIZE::default();
            let _ = GetTextExtentPoint32W(hdc, &wide_match, &mut match_size);

            let segment_x = start_x + prefix_size.cx;

            let out_rc = RECT {
                left: segment_x,
                top: rc.top,
                right: segment_x + match_size.cx,
                bottom: rc.bottom,
            };

            let _ = ExtTextOutW(
                hdc,
                segment_x,
                y,
                windows::Win32::Graphics::Gdi::ETO_CLIPPED | windows::Win32::Graphics::Gdi::ETO_OPAQUE,
                Some(&out_rc),
                PCWSTR(wide_match.as_ptr()),
                wide_match.len() as u32,
                None,
            );
        }
    }
}

pub fn lv_get_subitem_text(lv_hwnd: isize, item: i32, subitem: i32) -> String {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
    // LVITEMW partial for LVM_GETITEMTEXTW
    #[repr(C)]
    struct LvItemW {
        mask: u32,
        i_item: i32,
        i_sub_item: i32,
        state: u32,
        state_mask: u32,
        psz_text: windows::core::PWSTR,
        cch_text_max: i32,
        i_image: i32,
        l_param: isize,
        i_indent: i32,
        i_group_id: i32,
        c_columns: u32,
        pu_columns: *mut u32,
        pi_col_fmt: *mut i32,
        i_group: i32,
    }
    let mut buf = vec![0u16; 1024];
    let mut lv = LvItemW {
        mask: 0x0001, // LVIF_TEXT
        i_item: item,
        i_sub_item: subitem,
        state: 0,
        state_mask: 0,
        psz_text: windows::core::PWSTR(buf.as_mut_ptr()),
        cch_text_max: buf.len() as i32,
        i_image: 0,
        l_param: 0,
        i_indent: 0,
        i_group_id: 0,
        c_columns: 0,
        pu_columns: std::ptr::null_mut(),
        pi_col_fmt: std::ptr::null_mut(),
        i_group: 0,
    };
    unsafe {
        let _ = SendMessageW(
            HWND(lv_hwnd as _),
            0x1000 + 115, // LVM_GETITEMTEXTW = 0x1073
            WPARAM(item as usize),
            LPARAM(&mut lv as *mut LvItemW as isize),
        );
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..len])
    }
}

pub fn draw_highlighted_text(
    hdc: windows::Win32::Graphics::Gdi::HDC,
    text: &str,
    ranges: &[(usize, usize)],
    rc: &windows::Win32::Foundation::RECT,
    dark: bool,
    selected: bool,
    lv_hwnd: isize,
    align_right: bool,
) {
    use windows::Win32::Foundation::{COLORREF, RECT};
    use windows::Win32::Graphics::Gdi::{
        CreateSolidBrush, DeleteObject, ExtTextOutW, FillRect, GetTextExtentPoint32W, SetBkColor,
        SetBkMode, SetTextColor, ETO_CLIPPED, ETO_OPAQUE, HGDIOBJ, OPAQUE,
    };
    use windows::core::PCWSTR;
    use std::os::windows::ffi::OsStrExt;
    use std::ffi::OsStr;

    let (fg, bg) = if selected {
        unsafe {
            use windows::Win32::Graphics::Gdi::{
                GetSysColor, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_BTNFACE, COLOR_BTNTEXT,
            };
            use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;

            let has_focus = GetFocus().0 as isize == lv_hwnd;
            if has_focus {
                (
                    GetSysColor(COLOR_HIGHLIGHTTEXT),
                    GetSysColor(COLOR_HIGHLIGHT),
                )
            } else if dark {
                (
                    CLR_DARK_FG,
                    0x003F3F3F, // Match Explorer's inactive selection gray in dark mode
                )
            } else {
                (
                    GetSysColor(COLOR_BTNTEXT),
                    GetSysColor(COLOR_BTNFACE),
                )
            }
        }
    } else if dark {
        (CLR_DARK_FG, CLR_DARK_BG)
    } else {
        (CLR_LIGHT_FG, CLR_LIGHT_BG)
    };
    // Highlight colors (BGR)
    let (hl_fg, hl_bg) = if dark {
        (0x00000000, 0x0000D7FF) // black on gold
    } else {
        (0x00000000, 0x0080FFFF) // black on light yellow
    };

    let mut theme_drawn = false;
    if selected {
        use windows::core::HRESULT;
        use windows::core::w;

        #[link(name = "uxtheme")]
        unsafe extern "system" {
            fn OpenThemeData(hwnd: windows::Win32::Foundation::HWND, pszClassList: windows::core::PCWSTR) -> isize;
            fn CloseThemeData(hTheme: isize) -> HRESULT;
            fn DrawThemeBackground(
                hTheme: isize,
                hdc: windows::Win32::Graphics::Gdi::HDC,
                iPartId: i32,
                iStateId: i32,
                pRect: *const RECT,
                pClipRect: *const RECT,
            ) -> HRESULT;
        }

        unsafe {
            use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
            let has_focus = GetFocus().0 as isize == lv_hwnd;
            let htheme = OpenThemeData(windows::Win32::Foundation::HWND(lv_hwnd as _), w!("LISTVIEW"));
            if htheme != 0 {
                // LVP_LISTITEM = 1
                // Active selected: LVIS_SELECTED = 3
                // Inactive selected: LVIS_SELECTEDNOTFOCUS = 5
                let state = if has_focus { 3 } else { 5 };
                let _ = DrawThemeBackground(htheme, hdc, 1, state, rc, std::ptr::null());
                let _ = CloseThemeData(htheme);
                theme_drawn = true;
            }
        }
    }

    unsafe {
        if !theme_drawn {
            let brush = CreateSolidBrush(COLORREF(bg));
            let _ = FillRect(hdc, rc, brush);
            let _ = DeleteObject(HGDIOBJ(brush.0));
        }
        let _ = SetBkMode(hdc, OPAQUE);

        let mut x = if align_right {
            let wide_all: Vec<u16> = OsStr::new(text).encode_wide().collect();
            let mut size_all = windows::Win32::Foundation::SIZE::default();
            let _ = GetTextExtentPoint32W(hdc, &wide_all, &mut size_all);
            rc.right - size_all.cx - 6
        } else {
            rc.left + 4
        };
        let y = rc.top + 2;
        let mut pos = 0usize;
        let mut segments: Vec<(usize, usize, bool)> = Vec::new();
        for &(s, e) in ranges {
            if s >= text.len() || e > text.len() || s >= e {
                continue;
            }
            if pos < s {
                segments.push((pos, s, false));
            }
            segments.push((s, e, true));
            pos = e;
        }
        if pos < text.len() {
            segments.push((pos, text.len(), false));
        }
        if segments.is_empty() {
            segments.push((0, text.len(), false));
        }

        for (s, e, is_hl) in segments {
            let slice = &text[s..e];
            if slice.is_empty() {
                continue;
            }
            let wide: Vec<u16> = OsStr::new(slice).encode_wide().collect();
            let mut size = windows::Win32::Foundation::SIZE::default();
            let _ = GetTextExtentPoint32W(hdc, &wide, &mut size);
            if is_hl {
                let _ = SetTextColor(hdc, COLORREF(hl_fg));
                let _ = SetBkColor(hdc, COLORREF(hl_bg));
            } else {
                let _ = SetTextColor(hdc, COLORREF(fg));
                let _ = SetBkColor(hdc, COLORREF(bg));
            }
            let out_rc = RECT {
                left: x,
                top: rc.top,
                right: rc.right,
                bottom: rc.bottom,
            };
            let _ = ExtTextOutW(
                hdc,
                x,
                y,
                ETO_CLIPPED | ETO_OPAQUE,
                Some(&out_rc as *const RECT),
                PCWSTR(wide.as_ptr()),
                wide.len() as u32,
                None,
            );
            x += size.cx;
            if x >= rc.right {
                break;
            }
        }
    }
}

/// ListView NM_CUSTOMDRAW: content highlight + grid lines.
pub fn handle_listview_custom_draw(
    lparam: isize,
    lv_hwnd: isize,
    dark: bool,
    highlight: Option<&HighlightState>,
) -> Option<isize> {
    use windows::Win32::Foundation::{HWND, RECT, LPARAM, WPARAM, COLORREF};
    use windows::Win32::Graphics::Gdi::{
        CreatePen, SelectObject, MoveToEx, LineTo, DeleteObject, PS_SOLID, HDC, HGDIOBJ,
    };
    use windows::Win32::UI::Controls::NMLVCUSTOMDRAW;
    use windows::Win32::UI::WindowsAndMessaging::SendMessageW;

    const NM_CUSTOMDRAW: u32 = (-12i32) as u32;
    const CDDS_PREPAINT: u32 = 0x00000001;
    const CDDS_POSTPAINT: u32 = 0x00000002;
    const CDDS_ITEMPREPAINT: u32 = 0x00010001;
    const CDDS_SUBITEMPREPAINT: u32 = 0x00030001;
    const CDRF_DODEFAULT: isize = 0x00000000;
    const CDRF_NOTIFYPOSTPAINT: isize = 0x00000010;
    const CDRF_NOTIFYITEMDRAW: isize = 0x00000020;
    const CDRF_NOTIFYSUBITEMDRAW: isize = 0x00000020;
    const CDRF_SKIPDEFAULT: isize = 0x00000004;
    const CDRF_NEWFONT: isize = 0x00000002;
    const CDIS_SELECTED: u32 = 0x0001;
    const GRID_DARK: u32 = 0x00404040;
    const GRID_LIGHT: u32 = 0x00C0C0C0;

    let nm = lparam as *mut NMLVCUSTOMDRAW;
    if nm.is_null() {
        return None;
    }
    let cd = unsafe { &mut *nm };
    if cd.nmcd.hdr.hwndFrom.0 as isize != lv_hwnd {
        return None;
    }
    if cd.nmcd.hdr.code != NM_CUSTOMDRAW {
        return None;
    }

    let stage = cd.nmcd.dwDrawStage.0;

    match stage {
        CDDS_PREPAINT => Some(CDRF_NOTIFYITEMDRAW | CDRF_NOTIFYPOSTPAINT),
        CDDS_ITEMPREPAINT => {
            // Default colors for non-content columns
            if dark {
                cd.clrText = COLORREF(CLR_DARK_FG);
                cd.clrTextBk = COLORREF(CLR_DARK_BG);
            }
            Some(CDRF_NOTIFYSUBITEMDRAW | CDRF_NEWFONT)
        }
        s if s == CDDS_SUBITEMPREPAINT => {
            let selected = (cd.nmcd.uItemState.0 & CDIS_SELECTED) != 0;

            // If the row is selected, let the OS handle native visual style background and text rendering.
            // This guarantees 100% native selection display (e.g. explorer translucency) without color mismatch.
            if selected {
                if dark {
                    cd.clrText = COLORREF(CLR_DARK_FG);
                    cd.clrTextBk = COLORREF(CLR_DARK_BG);
                }
                // If content column (2) and there is active query highlights, request POSTPAINT to overlay highlights on selection
                if cd.iSubItem == 2 && highlight.is_some() {
                    return Some(CDRF_DODEFAULT | CDRF_NOTIFYPOSTPAINT);
                }
                return Some(CDRF_DODEFAULT);
            }

            // Custom draw only for the content column (2) when NOT selected, to highlight match keywords.
            if cd.iSubItem == 2
                && let Some(hl) = highlight {
                    let item = cd.nmcd.dwItemSpec as i32;
                    let text = lv_get_subitem_text(lv_hwnd, item, 2);
                    let ranges = find_highlight_ranges(&text, hl);
                    let hdc = HDC(cd.nmcd.hdc.0);
                    let rc = cd.nmcd.rc;
                    draw_highlighted_text(hdc, &text, &ranges, &rc, dark, false, lv_hwnd, false);
                    return Some(CDRF_SKIPDEFAULT);
                }

            if dark {
                cd.clrText = COLORREF(CLR_DARK_FG);
                cd.clrTextBk = COLORREF(CLR_DARK_BG);
            }
            Some(CDRF_NEWFONT)
        }
        s if s == 0x00030002 => { // CDDS_SUBITEMPOSTPAINT
            if cd.iSubItem == 2
                && let Some(hl) = highlight {
                    let item = cd.nmcd.dwItemSpec as i32;
                    let text = lv_get_subitem_text(lv_hwnd, item, 2);
                    let ranges = find_highlight_ranges(&text, hl);
                    if !ranges.is_empty() {
                        let hdc = HDC(cd.nmcd.hdc.0);
                        let rc = cd.nmcd.rc;
                        let selected = (cd.nmcd.uItemState.0 & CDIS_SELECTED) != 0;
                        draw_postpaint_highlights(hdc, &text, &ranges, &rc, dark, selected);
                    }
                }
            Some(CDRF_DODEFAULT)
        }
        CDDS_POSTPAINT => {
            let hwnd = HWND(lv_hwnd as _);
            let hdc = HDC(cd.nmcd.hdc.0 as _);
            let grid_clr = if dark { GRID_DARK } else { GRID_LIGHT };

            unsafe {
                let mut client = RECT::default();
                let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut client);

                let header = SendMessageW(hwnd, 0x1000 + 31 /* LVM_GETHEADER */, WPARAM(0), LPARAM(0));
                let mut header_h = 0i32;
                if header.0 != 0 {
                    let mut header_rc = RECT::default();
                    let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(
                        HWND(header.0 as _),
                        &mut header_rc,
                    );
                    header_h = header_rc.bottom - header_rc.top;
                }

                let top_idx = SendMessageW(hwnd, 0x1000 + 39 /* LVM_GETTOPINDEX */, WPARAM(0), LPARAM(0)).0 as i32;
                let count = SendMessageW(hwnd, 0x1000 + 4 /* LVM_GETITEMCOUNT */, WPARAM(0), LPARAM(0)).0 as i32;
                let per_page = SendMessageW(hwnd, 0x1000 + 40 /* LVM_GETCOUNTPERPAGE */, WPARAM(0), LPARAM(0)).0 as i32;

                let pen = CreatePen(PS_SOLID, 1, COLORREF(grid_clr));
                let old = SelectObject(hdc, HGDIOBJ(pen.0));

                if count > 0 {
                    let last = (top_idx + per_page + 1).min(count);
                    for idx in top_idx..last {
                        let mut rc = RECT {
                            left: 0,
                            top: 0,
                            right: 0,
                            bottom: 0,
                        };
                        let ok = SendMessageW(
                            hwnd,
                            0x1000 + 14, /* LVM_GETITEMRECT */
                            WPARAM(idx as usize),
                            LPARAM(&mut rc as *mut RECT as isize),
                        );
                        if ok.0 == 0 {
                            continue;
                        }
                        let y = rc.bottom - 1;
                        if y > header_h && y < client.bottom {
                            let _ = MoveToEx(hdc, 0, y, None);
                            let _ = LineTo(hdc, client.right, y);
                        }
                    }
                }

                let mut x = 0i32;
                // Only draw separators between columns (0 and 1)
                // Exclude column 2 (index 2) right border to avoid extra line near scrollbar
                for col in 0..2 {
                    let w = SendMessageW(hwnd, 0x1000 + 29 /* LVM_GETCOLUMNWIDTH */, WPARAM(col), LPARAM(0)).0 as i32;
                    if w <= 0 {
                        break;
                    }
                    x += w;
                    if x >= client.right {
                        break;
                    }
                    let _ = MoveToEx(hdc, x, header_h, None);
                    let _ = LineTo(hdc, x, client.bottom);
                }

                let _ = SelectObject(hdc, old);
                let _ = DeleteObject(HGDIOBJ(pen.0));
            }
            Some(CDRF_DODEFAULT)
        }
        _ => None,
    }
}

/// Handle WM_CTLCOLOR* / WM_ERASEBKGND for dark/light painting. Returns Some(brush) or None.
/// `hwnd` is the window that received the message (needed for WM_ERASEBKGND).
pub fn handle_theme_color_msg(
    hwnd: isize,
    msg: u32,
    wparam: usize,
    brushes: &ThemeBrushes,
    dark: bool,
) -> Option<isize> {
    use windows::Win32::Graphics::Gdi::{
        SetTextColor, SetBkColor, SetBkMode, FillRect, OPAQUE, HDC,
    };
    use windows::Win32::Foundation::{COLORREF, RECT, HWND};
    use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

    let (fg, bg_clr, brush) = if dark {
        (CLR_DARK_FG, CLR_DARK_BG, brushes.bg(true))
    } else {
        (CLR_LIGHT_FG, CLR_LIGHT_BG, brushes.bg(false))
    };
    let edit_brush = brushes.edit(dark);
    let edit_bg = if dark { CLR_DARK_EDIT } else { CLR_LIGHT_EDIT };

    match msg {
        0x0014 => {
            // WM_ERASEBKGND — fill client area with theme brush
            let hdc = HDC(wparam as _);
            let wh = HWND(hwnd as _);
            let mut rc = RECT::default();
            unsafe {
                let _ = GetClientRect(wh, &mut rc);
                let _ = FillRect(hdc, &rc, brush);
            }
            Some(1)
        }
        0x0138 => {
            // WM_CTLCOLORSTATIC (Labels/CheckBoxes/etc.)
            // Always use solid theme brush — transparent/NULL_BRUSH leaves system
            // light background because parent erase does not paint under children.
            let hdc = HDC(wparam as _);
            unsafe {
                let _ = SetTextColor(hdc, COLORREF(fg));
                let _ = SetBkColor(hdc, COLORREF(bg_clr));
                let _ = SetBkMode(hdc, OPAQUE);
            }
            Some(brush.0 as isize)
        }
        0x0135 => {
            // WM_CTLCOLORBTN (Buttons)
            let hdc = HDC(wparam as _);
            unsafe {
                let _ = SetTextColor(hdc, COLORREF(fg));
                let _ = SetBkColor(hdc, COLORREF(bg_clr));
                let _ = SetBkMode(hdc, OPAQUE);
            }
            Some(brush.0 as isize)
        }
        0x0133 | 0x0134 => {
            // WM_CTLCOLOREDIT / WM_CTLCOLORLISTBOX
            let hdc = HDC(wparam as _);
            unsafe {
                let _ = SetTextColor(hdc, COLORREF(fg));
                let _ = SetBkColor(hdc, COLORREF(edit_bg));
                let _ = SetBkMode(hdc, OPAQUE);
            }
            Some(edit_brush.0 as isize)
        }
        _ => None,
    }
}

pub fn handle_treeview_custom_draw(
    lparam: isize,
    tv_hwnd: isize,
    dark: bool,
) -> Option<isize> {
    use windows::Win32::Foundation::COLORREF;
    use windows::Win32::UI::Controls::NMTVCUSTOMDRAW;
    use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;

    const NM_CUSTOMDRAW: u32 = (-12i32) as u32;
    const CDDS_PREPAINT: u32 = 0x00000001;
    const CDDS_ITEMPREPAINT: u32 = 0x00010001;
    const CDRF_DODEFAULT: isize = 0x00000000;
    const CDRF_NOTIFYITEMDRAW: isize = 0x00000020;
    const CDRF_NEWFONT: isize = 0x00000002;
    const CDIS_SELECTED: u32 = 0x0001;

    let nm = lparam as *mut NMTVCUSTOMDRAW;
    if nm.is_null() {
        return None;
    }
    let cd = unsafe { &mut *nm };
    if cd.nmcd.hdr.hwndFrom.0 as isize != tv_hwnd {
        return None;
    }
    if cd.nmcd.hdr.code != NM_CUSTOMDRAW {
        return None;
    }

    let stage = cd.nmcd.dwDrawStage.0;

    match stage {
        CDDS_PREPAINT => Some(CDRF_NOTIFYITEMDRAW),
        CDDS_ITEMPREPAINT => {
            let selected = (cd.nmcd.uItemState.0 & CDIS_SELECTED) != 0;
            if selected {
                // Clear selection state so control doesn't paint default bright blue highlight
                cd.nmcd.uItemState.0 &= !CDIS_SELECTED;

                let has_focus = unsafe { GetFocus().0 as isize == tv_hwnd };
                if dark {
                    if has_focus {
                        cd.clrTextBk = COLORREF(0x008E5B26); // Active selection: distinct steel blue (RGB: 38, 91, 142)
                        cd.clrText = COLORREF(0x00FFFFFF);   // White text
                    } else {
                        cd.clrTextBk = COLORREF(0x003A3A3A); // Inactive selection: charcoal gray (RGB: 58, 58, 58)
                        cd.clrText = COLORREF(0x00CCCCCC);   // Light gray text
                    }
                } else {
                    if has_focus {
                        cd.clrTextBk = COLORREF(0x00FAD7B4); // Active selection: distinct soft sky blue (RGB: 180, 215, 250)
                        cd.clrText = COLORREF(0x00000000);   // Black text
                    } else {
                        cd.clrTextBk = COLORREF(0x00E1E1E1); // Inactive selection: light gray (RGB: 225, 225, 225)
                        cd.clrText = COLORREF(0x00555555);   // Medium gray text
                    }
                }
            } else {
                if dark {
                    cd.clrTextBk = COLORREF(CLR_DARK_BG);
                    cd.clrText = COLORREF(CLR_DARK_FG);
                } else {
                    cd.clrTextBk = COLORREF(CLR_LIGHT_BG);
                    cd.clrText = COLORREF(CLR_LIGHT_FG);
                }
            }
            Some(CDRF_NEWFONT)
        }
        _ => Some(CDRF_DODEFAULT),
    }
}
