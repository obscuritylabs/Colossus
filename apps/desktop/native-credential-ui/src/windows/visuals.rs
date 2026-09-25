//! App palette and owned GDI resources; credential text is never painted here.

use super::wide;
use crate::{ColorScheme, DialogAppearance, appearance::Palette};
use windows_sys::Win32::{
    Foundation::{COLORREF, HWND, RECT},
    Graphics::{
        Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute},
        Gdi::{
            CLEARTYPE_QUALITY, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW,
            COLOR_WINDOWTEXT, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, DeleteObject,
            FW_NORMAL, FW_SEMIBOLD, GetSysColor, HBRUSH, HFONT,
        },
    },
    UI::{
        Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
        HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow},
        WindowsAndMessaging::{
            SPI_GETHIGHCONTRAST, SystemParametersInfoW, WS_CAPTION, WS_EX_DLGMODALFRAME, WS_SYSMENU,
        },
    },
};

pub(super) const WIDTH: i32 = 560;
pub(super) const HEIGHT: i32 = 324;

pub(super) struct Visuals {
    pub appearance: DialogAppearance,
    pub dpi: u32,
    pub colors: Palette,
    pub high_contrast: bool,
    pub surface: HBRUSH,
    pub control: HBRUSH,
    pub heading_font: HFONT,
    pub body_font: HFONT,
    pub caption_font: HFONT,
}

impl Visuals {
    pub(super) unsafe fn new(parent: HWND, appearance: DialogAppearance) -> Self {
        let dpi = unsafe { GetDpiForWindow(parent) }.max(96);
        unsafe { Self::for_dpi(appearance, dpi) }
    }

    pub(super) unsafe fn for_dpi(appearance: DialogAppearance, dpi: u32) -> Self {
        let mut contrast = HIGHCONTRASTW {
            cbSize: u32::try_from(size_of::<HIGHCONTRASTW>()).unwrap(),
            ..Default::default()
        };
        let high_contrast = unsafe {
            SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                contrast.cbSize,
                (&raw mut contrast).cast(),
                0,
            )
        } != 0
            && contrast.dwFlags & HCF_HIGHCONTRASTON != 0;
        let mut colors = appearance.palette();
        if high_contrast {
            unsafe {
                colors.surface = color(GetSysColor(COLOR_WINDOW));
                colors.control = colors.surface;
                colors.text = color(GetSysColor(COLOR_WINDOWTEXT));
                colors.strong = colors.text;
                colors.border = colors.text;
                colors.muted = color(GetSysColor(COLOR_GRAYTEXT));
                colors.accent = color(GetSysColor(COLOR_HIGHLIGHT));
                colors.accent_hover = colors.accent;
                colors.on_accent = color(GetSysColor(COLOR_HIGHLIGHTTEXT));
                colors.hover = colors.surface;
                colors.danger = colors.text;
                colors.focus = colors.text;
            }
        }
        let mut result = Self {
            appearance,
            dpi,
            colors,
            high_contrast,
            surface: unsafe { CreateSolidBrush(color(colors.surface)) },
            control: unsafe { CreateSolidBrush(color(colors.control)) },
            heading_font: std::ptr::null_mut(),
            body_font: std::ptr::null_mut(),
            caption_font: std::ptr::null_mut(),
        };
        result.heading_font = unsafe { result.font(20, FW_SEMIBOLD) };
        result.body_font = unsafe { result.font(14, FW_NORMAL) };
        result.caption_font = unsafe { result.font(12, FW_NORMAL) };
        result
    }

    pub(super) fn px(&self, logical: i32) -> i32 {
        let scaled = i64::from(logical)
            * i64::from(self.dpi)
            * i64::from(self.appearance.text_size.root_pixels());
        i32::try_from((scaled + 768) / 1536).unwrap_or(i32::MAX)
    }

    pub(super) fn outer_size(&self) -> (i32, i32) {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: self.px(WIDTH),
            bottom: self.px(HEIGHT),
        };
        unsafe {
            AdjustWindowRectExForDpi(
                &raw mut rect,
                WS_CAPTION | WS_SYSMENU,
                0,
                WS_EX_DLGMODALFRAME,
                self.dpi,
            );
        }
        (rect.right - rect.left, rect.bottom - rect.top)
    }

    pub(super) fn available(&self) -> bool {
        [
            self.surface,
            self.control,
            self.heading_font,
            self.body_font,
            self.caption_font,
        ]
        .iter()
        .all(|object| !object.is_null())
    }

    pub(super) unsafe fn apply_titlebar(&self, window: HWND) {
        let dark =
            i32::from(self.appearance.color_scheme == ColorScheme::Dark && !self.high_contrast);
        // Older Windows builds can ignore this optional caption appearance hint.
        unsafe {
            DwmSetWindowAttribute(
                window,
                DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                (&raw const dark).cast(),
                4,
            );
        }
    }

    unsafe fn font(&self, pixels: i32, weight: u32) -> HFONT {
        unsafe {
            CreateFontW(
                -self.px(pixels),
                0,
                0,
                0,
                i32::try_from(weight).unwrap(),
                0,
                0,
                0,
                u32::from(DEFAULT_CHARSET),
                0,
                0,
                u32::from(CLEARTYPE_QUALITY),
                0,
                wide("Segoe UI").as_ptr(),
            )
        }
    }
}

impl Drop for Visuals {
    fn drop(&mut self) {
        // Controls have released these resources before their session is dropped.
        unsafe {
            for object in [
                self.surface,
                self.control,
                self.heading_font,
                self.body_font,
                self.caption_font,
            ] {
                if !object.is_null() {
                    DeleteObject(object);
                }
            }
        }
    }
}

pub(super) const fn color(rgb: u32) -> COLORREF {
    ((rgb & 0xff) << 16) | (rgb & 0xff00) | ((rgb >> 16) & 0xff)
}
