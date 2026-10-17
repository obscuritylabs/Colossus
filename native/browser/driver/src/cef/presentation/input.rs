//! Closed native input conversion; text owner remains alive through the C ABI call.
use crate::ffi;
use colossus_browser_presentation::{Input, PresentationError};

pub(super) fn map(
    input: &Input,
    modifiers: u32,
) -> Result<(ffi::PresentationInput, Vec<u16>), PresentationError> {
    if modifiers & !0x1fff != 0 {
        return Err(PresentationError::Invalid);
    }
    let mut native = ffi::PresentationInput {
        version: 1,
        kind: 0,
        modifiers,
        x: 0,
        y: 0,
        button: 0,
        wheel_x: 0,
        wheel_y: 0,
        key_code: 0,
        text: std::ptr::null(),
        text_units: 0,
    };
    let mut text = Vec::new();
    match input {
        Input::MouseMove { x, y } => {
            native.kind = 1;
            native.x = *x as i32;
            native.y = *y as i32;
        }
        Input::MouseButton {
            x,
            y,
            button,
            pressed,
        } => {
            native.kind = if *pressed { 2 } else { 3 };
            native.x = *x as i32;
            native.y = *y as i32;
            native.button = i32::from(*button);
        }
        Input::MouseWheel {
            x,
            y,
            delta_x,
            delta_y,
        } => {
            native.kind = 4;
            native.x = *x as i32;
            native.y = *y as i32;
            native.wheel_x = *delta_x;
            native.wheel_y = *delta_y;
        }
        Input::Key { code, pressed } => {
            native.kind = if *pressed { 5 } else { 6 };
            native.key_code = i32::from(*code);
        }
        Input::Character { text: value } => {
            text.extend(value.encode_utf16());
            native.kind = if text.len() == 1 { 7 } else { 8 };
            if text.len() == 1 {
                native.key_code = i32::from(text[0]);
            }
        }
        Input::ImeCommit { text: value } => {
            text.extend(value.encode_utf16());
            native.kind = 8;
        }
        Input::ImeCancel => native.kind = 9,
    }
    native.text_units = text.len();
    if !text.is_empty() {
        native.text = text.as_ptr();
    }
    Ok((native, text))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bmp_character_has_key_code_and_supplementary_character_uses_ime() {
        let (bmp, text) = map(&Input::Character { text: "A".into() }, 0).unwrap();
        assert_eq!((bmp.kind, bmp.key_code, bmp.text_units), (7, 65, 1));
        assert_eq!(text, vec![65]);
        let (supplementary, text) = map(
            &Input::Character {
                text: "😀".into()
            },
            0,
        )
        .unwrap();
        assert_eq!(
            (
                supplementary.kind,
                supplementary.key_code,
                supplementary.text_units
            ),
            (8, 0, 2)
        );
        assert_eq!(text, "😀".encode_utf16().collect::<Vec<_>>());
    }
}
