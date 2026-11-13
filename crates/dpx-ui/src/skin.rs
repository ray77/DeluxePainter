//! Which skin draws the interface and which theme the Modern skin uses (ADR-47). Model types
//! only: the Classic skin is this crate, the Modern skin is drawn elsewhere, and both share
//! one [`UiState`](crate::UiState).

/// The skin that draws the main window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum UiStyle {
    /// The pixel UI of ADR-3; what a first start shows.
    #[default]
    Classic,
    /// The vector UI of ADR-47; only the picture is pixels.
    Modern,
}

impl UiStyle {
    /// Both skins.
    pub const ALL: [UiStyle; 2] = [UiStyle::Classic, UiStyle::Modern];

    /// Stable name used in the settings file.
    pub fn key(self) -> &'static str {
        match self {
            UiStyle::Classic => "classic",
            UiStyle::Modern => "modern",
        }
    }

    /// The skin with settings-file name `key`.
    pub fn from_key(key: &str) -> Option<UiStyle> {
        Self::ALL.into_iter().find(|s| s.key() == key)
    }

    /// The other skin (the switch key and Prefs > Modern UI).
    pub fn toggled(self) -> UiStyle {
        match self {
            UiStyle::Classic => UiStyle::Modern,
            UiStyle::Modern => UiStyle::Classic,
        }
    }
}

/// The colour scheme of the Modern skin. The Classic skin has none: its pens come from the
/// picture's palette.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ThemeChoice {
    /// Follow the operating system's dark or light setting.
    #[default]
    System,
    Dark,
    Light,
}

impl ThemeChoice {
    /// Every choice, in menu order.
    pub const ALL: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Dark, ThemeChoice::Light];

    /// Stable name used in the settings file.
    pub fn key(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Dark => "dark",
            ThemeChoice::Light => "light",
        }
    }

    /// The choice with settings-file name `key`.
    pub fn from_key(key: &str) -> Option<ThemeChoice> {
        Self::ALL.into_iter().find(|t| t.key() == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_first_start_is_classic_and_follows_the_system() {
        assert_eq!(UiStyle::default(), UiStyle::Classic);
        assert_eq!(ThemeChoice::default(), ThemeChoice::System);
    }

    #[test]
    fn keys_round_trip_and_unknown_names_are_refused() {
        for s in UiStyle::ALL {
            assert_eq!(UiStyle::from_key(s.key()), Some(s));
        }
        for t in ThemeChoice::ALL {
            assert_eq!(ThemeChoice::from_key(t.key()), Some(t));
        }
        assert_eq!(UiStyle::from_key("Modern"), None);
        assert_eq!(UiStyle::from_key(""), None);
        assert_eq!(ThemeChoice::from_key("auto"), None);
    }

    #[test]
    fn toggling_twice_returns() {
        for s in UiStyle::ALL {
            assert_ne!(s.toggled(), s);
            assert_eq!(s.toggled().toggled(), s);
        }
    }
}
