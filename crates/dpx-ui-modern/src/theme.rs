//! Theme tokens of the Modern skin (ADR-47): fixed colours, never pens of the picture's
//! palette, so the interface looks the same over every picture and the picture never changes
//! with the theme.

use dpx_ui::ThemeChoice;

use crate::paint::Color;

/// The two colour schemes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThemeKind {
    Dark,
    Light,
}

impl ThemeKind {
    pub const ALL: [ThemeKind; 2] = [ThemeKind::Dark, ThemeKind::Light];

    /// The scheme a Prefs choice means. `system_dark` is what the operating system reports
    /// (`None` when it reports nothing: Dark then).
    pub fn resolve(choice: ThemeChoice, system_dark: Option<bool>) -> ThemeKind {
        match choice {
            ThemeChoice::Dark => ThemeKind::Dark,
            ThemeChoice::Light => ThemeKind::Light,
            ThemeChoice::System if system_dark == Some(false) => ThemeKind::Light,
            ThemeChoice::System => ThemeKind::Dark,
        }
    }
}

/// The colours of one scheme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub kind: ThemeKind,
    /// The picture viewport around the picture.
    pub canvas: Color,
    /// Bar, dock and layer column.
    pub panel: Color,
    /// Buttons, menu cards, tooltips.
    pub raised: Color,
    /// A button under the pointer.
    pub hover: Color,
    /// Separators and card borders.
    pub hairline: Color,
    /// Labels and icons.
    pub text: Color,
    /// Idle menu titles, shortcuts, secondary status text.
    pub text_dim: Color,
    /// Ghosted items and buttons.
    pub disabled: Color,
    /// Selection, menu hover, toggle rings.
    pub accent: Color,
    /// Ink on the accent.
    pub on_accent: Color,
    /// Over the chrome while a requester is open.
    pub scrim: Color,
    /// Under menu cards and around the picture.
    pub shadow: Color,
}

impl Theme {
    pub const fn dark() -> Theme {
        Theme {
            kind: ThemeKind::Dark,
            canvas: Color::hex(0x16171A),
            panel: Color::hex(0x202227),
            raised: Color::hex(0x30333A),
            hover: Color::hex(0x3D414A),
            hairline: Color::hex(0x3A3D45),
            text: Color::hex(0xE4E6EB),
            text_dim: Color::hex(0x9A9FAA),
            disabled: Color::hex(0x5C606A),
            accent: Color::hex(0xFF8A1F),
            on_accent: Color::hex(0x1A1206),
            scrim: Color::hex(0x000000).with_opacity(45),
            shadow: Color::hex(0x000000).with_opacity(40),
        }
    }

    pub const fn light() -> Theme {
        Theme {
            kind: ThemeKind::Light,
            canvas: Color::hex(0xD6D9DF),
            panel: Color::hex(0xF0F1F4),
            raised: Color::hex(0xFFFFFF),
            hover: Color::hex(0xE3E6EC),
            hairline: Color::hex(0xC9CDD5),
            text: Color::hex(0x1C1E24),
            text_dim: Color::hex(0x5E636E),
            disabled: Color::hex(0xA4A8B1),
            accent: Color::hex(0xD66214),
            // Dark ink as in the dark scheme: white on this orange reaches only 3.7:1.
            on_accent: Color::hex(0x1A1206),
            scrim: Color::hex(0x14161C).with_opacity(30),
            shadow: Color::hex(0x000000).with_opacity(18),
        }
    }

    pub const fn of(kind: ThemeKind) -> Theme {
        match kind {
            ThemeKind::Dark => Theme::dark(),
            ThemeKind::Light => Theme::light(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_choice_follows_the_system_and_is_dark_without_an_answer() {
        use ThemeChoice::*;
        assert_eq!(ThemeKind::resolve(System, Some(true)), ThemeKind::Dark);
        assert_eq!(ThemeKind::resolve(System, Some(false)), ThemeKind::Light);
        assert_eq!(ThemeKind::resolve(System, None), ThemeKind::Dark);
        for system in [None, Some(true), Some(false)] {
            assert_eq!(ThemeKind::resolve(Dark, system), ThemeKind::Dark);
            assert_eq!(ThemeKind::resolve(Light, system), ThemeKind::Light);
        }
    }

    /// ADR-47: text on the panel and ink on the accent reach a contrast of 4.5:1. Text on
    /// buttons and dim text on the panel are held to the same bar here.
    #[test]
    fn text_is_legible_on_its_surfaces() {
        for kind in ThemeKind::ALL {
            let t = Theme::of(kind);
            assert_eq!(t.kind, kind);
            for (name, ink, ground) in [
                ("text on panel", t.text, t.panel),
                ("text on raised", t.text, t.raised),
                ("text on hover", t.text, t.hover),
                ("dim text on panel", t.text_dim, t.panel),
                ("ink on accent", t.on_accent, t.accent),
            ] {
                let ratio = ink.contrast(ground);
                assert!(ratio >= 4.5, "{kind:?}: {name} is only {ratio:.2}:1");
            }
            // The accent must stand out from the surfaces it marks.
            for (name, ground) in [("panel", t.panel), ("raised", t.raised)] {
                let ratio = t.accent.contrast(ground);
                assert!(
                    ratio >= 3.0,
                    "{kind:?}: accent on {name} is only {ratio:.2}:1"
                );
            }
        }
    }

    #[test]
    fn surfaces_are_opaque_and_overlays_are_not() {
        for kind in ThemeKind::ALL {
            let t = Theme::of(kind);
            for c in [
                t.canvas, t.panel, t.raised, t.hover, t.hairline, t.text, t.accent,
            ] {
                assert_eq!(c.a, 255);
            }
            assert!(t.scrim.a > 0 && t.scrim.a < 255);
            assert!(t.shadow.a > 0 && t.shadow.a < 255);
        }
    }
}
