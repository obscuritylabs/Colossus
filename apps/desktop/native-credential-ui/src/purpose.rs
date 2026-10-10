//! Native dialog purpose and fixed, non-secret copy.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Purpose {
    Token,
    Pkcs12Password,
}

impl Purpose {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Token => "Save a Colossus credential",
            Self::Pkcs12Password => "Unlock PKCS#12 identity",
        }
    }

    #[cfg(any(windows, target_os = "macos"))]
    pub(crate) fn heading(self) -> &'static str {
        match self {
            Self::Token => "Save credential",
            Self::Pkcs12Password => "Unlock PKCS#12 identity",
        }
    }

    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::Token => "Your token is saved in the encrypted credential vault.",
            Self::Pkcs12Password => {
                "Enter the selected certificate package's passphrase. It is used only for native import and is not saved."
            }
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Token => "Token",
            Self::Pkcs12Password => "Passphrase",
        }
    }

    pub(crate) fn placeholder(self) -> &'static str {
        match self {
            Self::Token => "Paste your token",
            Self::Pkcs12Password => "Passphrase (leave empty if the package has none)",
        }
    }

    #[cfg(not(windows))]
    pub(crate) fn confirm(self) -> &'static str {
        match self {
            Self::Token => "Save",
            Self::Pkcs12Password => "Continue",
        }
    }

    pub(crate) fn validate(self, value: &str) -> Result<(), crate::validation::InputError> {
        match self {
            Self::Token => crate::validation::validate(value),
            Self::Pkcs12Password => crate::validation::validate_password(value),
        }
    }
}
