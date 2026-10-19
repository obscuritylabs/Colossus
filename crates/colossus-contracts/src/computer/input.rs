use super::{ComputerElementId, ComputerValidationError, ComputerWindowId};
use serde::{Deserialize, Deserializer, Serialize, de::Error};

/// Maximum UTF-8 bytes in one typed text action.
pub const COMPUTER_MAX_TEXT_BYTES: usize = 8192;

/// A local physical pixel or observation-bound native semantic element.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComputerTarget {
    /// Point inside the captured target window; placement is revalidated natively.
    Point {
        /// Local physical horizontal coordinate.
        x: i32,
        /// Local physical vertical coordinate.
        y: i32,
    },
    /// Native adapter resolves this observed element to a fenced physical point.
    Element {
        /// Exact current observation's opaque element.
        element_id: ComputerElementId,
    },
}

/// Closed supported mouse buttons. Button-down state never crosses calls.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerMouseButton {
    /// Primary mouse button.
    Left,
    /// Context mouse button.
    Right,
    /// Middle mouse button.
    Middle,
}

macro_rules! computer_input {
    ($($variant:ident { $($(#[$doc:meta])* $field:ident: $ty:ty),* $(,)? }),* $(,)?) => {
        /// One balanced native input action, requiring a fresh consumed observation.
        #[derive(Clone, Eq, PartialEq, Serialize)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        pub enum ComputerInput {
            $(#[doc = concat!("Bounded native ", stringify!($variant), " action.")]
              $variant { $($(#[$doc])* $field: $ty),* }),*
        }

        impl std::fmt::Debug for ComputerInput {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                let kind = match self { $(Self::$variant { .. } => stringify!($variant)),* };
                formatter.debug_struct("ComputerInput").field("kind", &kind).finish_non_exhaustive()
            }
        }

        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
        enum ComputerInputWire { $($variant { $($field: $ty),* }),* }

        impl<'de> Deserialize<'de> for ComputerInput {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let input = match ComputerInputWire::deserialize(deserializer)? {
                    $(ComputerInputWire::$variant { $($field),* } => Self::$variant { $($field),* }),*
                };
                input.validate().map_err(D::Error::custom)?;
                Ok(input)
            }
        }
    };
}

computer_input! {
    Focus { /// Selected root or positively owned same-app modal.
        window_id: ComputerWindowId },
    Move { /// Local physical horizontal coordinate.
        x: i32, /// Local physical vertical coordinate.
        y: i32 },
    Click { /// Fresh point or semantic target.
        target: ComputerTarget, /// Mouse button.
        button: ComputerMouseButton, /// One or two balanced clicks.
        count: u8 },
    Drag { /// Local starting horizontal coordinate.
        from_x: i32, /// Local starting vertical coordinate.
        from_y: i32, /// Local final horizontal coordinate.
        to_x: i32, /// Local final vertical coordinate.
        to_y: i32, /// One through 100 intermediate points in a single balanced native batch.
        steps: u16 },
    Scroll { /// Local physical horizontal coordinate.
        x: i32, /// Local physical vertical coordinate.
        y: i32, /// Horizontal wheel units, at most 1200 in magnitude.
        delta_x: i32, /// Vertical wheel units, at most 1200 in magnitude.
        delta_y: i32 },
    Type { /// Text at most 8192 UTF-8 bytes; never log this value.
        text: String },
    Press { /// At most five unique supported keys, with exactly one nonmodifier.
        keys: Vec<String> },
}

impl ComputerInput {
    /// Validate bounded payloads before effect authorization and again before native dispatch.
    pub fn validate(&self) -> Result<(), ComputerValidationError> {
        let valid = match self {
            Self::Focus { .. } => true,
            Self::Move { x, y } => point(*x, *y),
            Self::Click { target, count, .. } => {
                (1..=2).contains(count)
                    && match target {
                        ComputerTarget::Point { x, y } => point(*x, *y),
                        ComputerTarget::Element { .. } => true,
                    }
            }
            Self::Drag {
                from_x,
                from_y,
                to_x,
                to_y,
                steps,
            } => point(*from_x, *from_y) && point(*to_x, *to_y) && (1..=100).contains(steps),
            Self::Scroll {
                x,
                y,
                delta_x,
                delta_y,
            } => {
                point(*x, *y)
                    && (-1200..=1200).contains(delta_x)
                    && (-1200..=1200).contains(delta_y)
                    && (*delta_x != 0 || *delta_y != 0)
            }
            Self::Type { text } => {
                !text.is_empty()
                    && text.len() <= COMPUTER_MAX_TEXT_BYTES
                    && !text
                        .chars()
                        .any(|ch| ch.is_control() && !matches!(ch, '\t' | '\r' | '\n'))
            }
            Self::Press { keys } => valid_chord(keys),
        };
        if valid {
            Ok(())
        } else {
            Err(ComputerValidationError)
        }
    }
}

fn point(x: i32, y: i32) -> bool {
    x >= 0 && y >= 0
}

fn modifier(key: &str) -> bool {
    matches!(key, "Control" | "Alt" | "Shift")
}

/// Closed native key vocabulary; system-wide Windows keys are deliberately absent.
pub fn is_supported_computer_key(key: &str) -> bool {
    modifier(key)
        || matches!(
            key,
            "Enter"
                | "Tab"
                | "Escape"
                | "Backspace"
                | "Delete"
                | "Insert"
                | "Home"
                | "End"
                | "PageUp"
                | "PageDown"
                | "ArrowLeft"
                | "ArrowRight"
                | "ArrowUp"
                | "ArrowDown"
                | "Space"
                | "F1"
                | "F2"
                | "F3"
                | "F4"
                | "F5"
                | "F6"
                | "F7"
                | "F8"
                | "F9"
                | "F10"
                | "F11"
                | "F12"
        )
        || (key.len() == 1
            && key
                .bytes()
                .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit()))
}

fn valid_chord(keys: &[String]) -> bool {
    if keys.is_empty()
        || keys.len() > 5
        || keys.iter().any(|key| !is_supported_computer_key(key))
        || keys.iter().filter(|key| !modifier(key)).count() != 1
        || keys
            .iter()
            .enumerate()
            .any(|(index, key)| keys[..index].contains(key))
    {
        return false;
    }
    let has = |key: &str| keys.iter().any(|value| value == key);
    !(has("Alt") && ["Tab", "Escape", "F4"].iter().any(|key| has(key))
        || has("Control") && has("Escape")
        || has("Control") && has("Alt") && has("Delete"))
}
