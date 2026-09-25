//! Native control chrome. Only fixed labels and categorical status are drawn.

use super::{
    Session, controls,
    visuals::{Visuals, color},
    wide,
};
use std::ptr::{null, null_mut};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{
        BeginPaint, CreatePen, CreateSolidBrush, DT_CENTER, DT_SINGLELINE, DT_VCENTER,
        DeleteObject, DrawFocusRect, DrawTextW, EndPaint, FillRect, HDC, InvalidateRect,
        PAINTSTRUCT, PS_SOLID, RoundRect, SelectObject, SetBkColor, SetBkMode, SetTextColor,
        TRANSPARENT,
    },
    UI::{
        Controls::{DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_SELECTED, WM_MOUSELEAVE},
        Input::KeyboardAndMouse::{GetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent},
        WindowsAndMessaging::{
            GetClientRect, GetParent, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, WM_CTLCOLOREDIT,
            WM_CTLCOLORSTATIC, WM_DPICHANGED, WM_DRAWITEM, WM_ERASEBKGND, WM_KILLFOCUS,
            WM_MOUSEMOVE, WM_PAINT, WM_SETFOCUS, WM_SETTINGCHANGE, WM_SIZE, WM_THEMECHANGED,
        },
    },
};

pub(super) unsafe fn handle(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    pointer: *mut Session,
) -> Option<LRESULT> {
    match message {
        WM_ERASEBKGND => Some(1),
        WM_PAINT => {
            unsafe {
                paint(window, pointer);
            }
            Some(0)
        }
        WM_SIZE => {
            unsafe {
                controls::layout(window, pointer);
                InvalidateRect(window, null(), 1);
            }
            None
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            Some(unsafe { control_color(message, wparam as HDC, lparam as HWND, pointer) })
        }
        WM_DRAWITEM => {
            let item = unsafe { &*(lparam as *const DRAWITEMSTRUCT) };
            if item.hwndItem == unsafe { (*pointer).save }
                || item.hwndItem == unsafe { (*pointer).cancel }
            {
                unsafe {
                    button(item, pointer);
                }
                Some(1)
            } else {
                None
            }
        }
        WM_DPICHANGED => {
            unsafe {
                refresh(window, pointer, u32::try_from(wparam & 0xffff).unwrap());
            }
            let suggested = unsafe { &*(lparam as *const RECT) };
            let (width, height) = unsafe { (*pointer).visuals.outer_size() };
            unsafe {
                SetWindowPos(
                    window,
                    null_mut(),
                    suggested.left,
                    suggested.top,
                    width,
                    height,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            Some(0)
        }
        WM_SETTINGCHANGE | WM_THEMECHANGED => {
            unsafe {
                refresh(window, pointer, (*pointer).visuals.dpi);
            }
            None
        }
        _ => None,
    }
}

unsafe fn refresh(window: HWND, pointer: *mut Session, dpi: u32) {
    let appearance = unsafe { (*pointer).visuals.appearance };
    let replacement = unsafe { Visuals::for_dpi(appearance, dpi.max(48)) };
    if !replacement.available() {
        return;
    }
    // Replace every control font before releasing the old GDI resources.
    let previous = unsafe { std::ptr::replace(&raw mut (*pointer).visuals, replacement) };
    unsafe {
        controls::layout(window, pointer);
        (*pointer).visuals.apply_titlebar(window);
        InvalidateRect(window, null(), 1);
    }
    drop(previous);
}

unsafe fn control_color(message: u32, dc: HDC, control: HWND, pointer: *const Session) -> LRESULT {
    let session = unsafe { &*pointer };
    let visual = &session.visuals;
    let input = message == WM_CTLCOLOREDIT;
    let text = if control == session.heading || control == session.label {
        visual.colors.strong
    } else if input {
        visual.colors.text
    } else if control == session.error {
        visual.colors.danger
    } else {
        visual.colors.muted
    };
    unsafe {
        SetTextColor(dc, color(text));
        SetBkColor(
            dc,
            color(if input {
                visual.colors.control
            } else {
                visual.colors.surface
            }),
        );
        SetBkMode(dc, TRANSPARENT.cast_signed());
    }
    if input {
        visual.control as LRESULT
    } else {
        visual.surface as LRESULT
    }
}

unsafe fn paint(window: HWND, pointer: *const Session) {
    let visual = unsafe { &(*pointer).visuals };
    let mut paint = PAINTSTRUCT::default();
    let dc = unsafe { BeginPaint(window, &raw mut paint) };
    let mut bounds = RECT::default();
    unsafe {
        GetClientRect(window, &raw mut bounds);
        FillRect(dc, &raw const bounds, visual.surface);
    }
    let field = RECT {
        left: visual.px(28),
        top: visual.px(136),
        right: (bounds.right - visual.px(28)).max(visual.px(28) + 1),
        bottom: visual.px(180),
    };
    let border = if unsafe { (*pointer).has_error.get() } {
        visual.colors.danger
    } else if unsafe { GetFocus() == (*pointer).input } {
        visual.colors.focus
    } else {
        visual.colors.border
    };
    unsafe {
        rounded(
            dc,
            field,
            visual.px(7),
            visual.px(1).max(1),
            border,
            visual.colors.control,
        );
    }
    let divider = RECT {
        left: visual.px(28),
        top: visual.px(240),
        right: (bounds.right - visual.px(28)).max(visual.px(28) + 1),
        bottom: visual.px(240) + 1,
    };
    let brush = unsafe { CreateSolidBrush(color(visual.colors.border)) };
    unsafe {
        FillRect(dc, &raw const divider, brush);
        DeleteObject(brush);
        EndPaint(window, &raw const paint);
    }
}

unsafe fn button(item: &DRAWITEMSTRUCT, pointer: *const Session) {
    let session = unsafe { &*pointer };
    let visual = &session.visuals;
    let primary = item.hwndItem == session.save;
    let disabled = item.itemState & ODS_DISABLED != 0;
    let highlighted = item.itemState & ODS_SELECTED != 0 || session.hovered.get() == item.hwndItem;
    let fill = if disabled {
        visual.colors.hover
    } else if primary && highlighted {
        visual.colors.accent_hover
    } else if primary {
        visual.colors.accent
    } else if highlighted {
        visual.colors.hover
    } else {
        visual.colors.surface
    };
    let text = if disabled {
        visual.colors.muted
    } else if primary {
        visual.colors.on_accent
    } else {
        visual.colors.text
    };
    let edge = if primary && !disabled {
        fill
    } else {
        visual.colors.border
    };
    unsafe {
        FillRect(item.hDC, &raw const item.rcItem, visual.surface);
        rounded(
            item.hDC,
            item.rcItem,
            visual.px(7),
            visual.px(1).max(1),
            edge,
            fill,
        );
        SetBkMode(item.hDC, TRANSPARENT.cast_signed());
        SetTextColor(item.hDC, color(text));
    }
    let old_font = unsafe { SelectObject(item.hDC, visual.body_font) };
    let mut bounds = item.rcItem;
    unsafe {
        DrawTextW(
            item.hDC,
            wide(if primary { "&Save" } else { "&Cancel" }).as_ptr(),
            -1,
            &raw mut bounds,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        SelectObject(item.hDC, old_font);
    }
    if item.itemState & ODS_FOCUS != 0 {
        let inset = visual.px(4);
        bounds.left += inset;
        bounds.right -= inset;
        bounds.top += inset;
        bounds.bottom -= inset;
        unsafe {
            DrawFocusRect(item.hDC, &raw const bounds);
        }
    }
}

unsafe fn rounded(dc: HDC, rect: RECT, radius: i32, width: i32, edge: u32, fill: u32) {
    let pen = unsafe { CreatePen(PS_SOLID, width, color(edge)) };
    let brush = unsafe { CreateSolidBrush(color(fill)) };
    let old_pen = unsafe { SelectObject(dc, pen) };
    let old_brush = unsafe { SelectObject(dc, brush) };
    unsafe {
        RoundRect(
            dc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius * 2,
            radius * 2,
        );
        SelectObject(dc, old_brush);
        SelectObject(dc, old_pen);
        DeleteObject(brush);
        DeleteObject(pen);
    }
}

pub(super) unsafe fn invalidate_input(pointer: *const Session) {
    unsafe {
        InvalidateRect(GetParent((*pointer).input), null(), 0);
    }
}

pub(super) unsafe fn control_state(window: HWND, message: u32, pointer: *const Session) {
    if message == WM_SETFOCUS || message == WM_KILLFOCUS {
        unsafe {
            InvalidateRect(GetParent(window), null(), 0);
            InvalidateRect(window, null(), 0);
        }
    }
    if window == unsafe { (*pointer).input } {
        return;
    }
    if message == WM_MOUSEMOVE {
        unsafe {
            (*pointer).hovered.set(window);
            let mut tracking = TRACKMOUSEEVENT {
                cbSize: u32::try_from(size_of::<TRACKMOUSEEVENT>()).unwrap(),
                dwFlags: TME_LEAVE,
                hwndTrack: window,
                dwHoverTime: 0,
            };
            TrackMouseEvent(&raw mut tracking);
            InvalidateRect(window, null(), 0);
        }
    } else if message == WM_MOUSELEAVE {
        unsafe {
            (*pointer).hovered.set(null_mut());
            InvalidateRect(window, null(), 0);
        }
    }
}
