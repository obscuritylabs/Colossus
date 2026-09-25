//! Bounded, non-secret appearance metadata shared by the native adapters.

/// The app's resolved palette, with an OS fallback before preferences arrive.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ColorScheme {
    #[default]
    System,
    Dark,
    Light,
}

/// The same local text-size choices offered by Desktop appearance settings.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextSize {
    Compact,
    #[default]
    Comfortable,
    Large,
}

#[cfg(any(windows, target_os = "macos"))]
impl TextSize {
    pub(crate) const fn root_pixels(self) -> i32 {
        match self {
            Self::Compact => 15,
            Self::Comfortable => 16,
            Self::Large => 18,
        }
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn scale(self) -> f64 {
        f64::from(self.root_pixels()) / 16.0
    }
}

/// Appearance values never select a credential, path, endpoint or token value.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DialogAppearance {
    pub color_scheme: ColorScheme,
    pub text_size: TextSize,
}

#[derive(Clone, Copy)]
#[cfg(any(windows, target_os = "macos"))]
pub(crate) struct Palette {
    pub surface: u32,
    pub control: u32,
    pub text: u32,
    pub strong: u32,
    pub muted: u32,
    #[cfg(windows)]
    pub border: u32,
    pub accent: u32,
    #[cfg(windows)]
    pub accent_hover: u32,
    #[cfg(windows)]
    pub on_accent: u32,
    #[cfg(windows)]
    pub hover: u32,
    pub danger: u32,
    #[cfg(windows)]
    pub focus: u32,
}

#[cfg(any(windows, target_os = "macos"))]
impl DialogAppearance {
    pub(crate) const fn palette(self) -> Palette {
        match self.color_scheme {
            ColorScheme::Dark => Palette {
                surface: 0x11_1f_30,
                control: 0x09_16_24,
                text: 0xe8_ef_f8,
                strong: 0xf7_fa_ff,
                muted: 0x91_a2_b8,

                #[cfg(windows)]
                border: 0x2b_42_60,
                accent: 0x43_89_ff,
                #[cfg(windows)]
                accent_hover: 0x55_94_ff,

                #[cfg(windows)]
                on_accent: 0x07_10_1d,

                #[cfg(windows)]
                hover: 0x18_2b_41,

                danger: 0xff_ad_b3,
                #[cfg(windows)]
                focus: 0x45_c8_ed,
            },
            ColorScheme::System | ColorScheme::Light => Palette {
                surface: 0xff_ff_ff,
                control: 0xff_ff_ff,
                text: 0x25_36_4a,
                strong: 0x10_20_33,
                muted: 0x52_65_7a,

                #[cfg(windows)]
                border: 0xb8_c7_d8,
                accent: 0x25_63_d9,
                #[cfg(windows)]
                accent_hover: 0x1d_4f_b8,

                #[cfg(windows)]
                on_accent: 0xff_ff_ff,

                #[cfg(windows)]
                hover: 0xe6_ed_f6,

                danger: 0x96_2d_3a,
                #[cfg(windows)]
                focus: 0x08_7d_a1,
            },
        }
    }
}
