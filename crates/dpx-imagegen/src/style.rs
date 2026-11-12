//! The picture style (ADR-45): a sentence added to the user's description before it is sent,
//! so the generator draws something that survives the reduction to a few colours. The GUI's
//! Style option and, later, MCP use the same text.

use serde::{Deserialize, Serialize};

/// The style asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    /// Classic pixel art with a limited palette (the default).
    #[default]
    PixelArt,
    /// A painted illustration with flat colour areas.
    Painted,
    /// The description is sent as typed.
    None,
}

impl Style {
    /// The sentence added for `colors` palette entries; empty for [`Style::None`].
    pub fn suffix(self, colors: u16) -> String {
        match self {
            Style::PixelArt => format!(
                "Style: classic Amiga-era pixel art, a limited palette of about {colors} \
                 colours, crisp hard-edged pixels, no anti-aliasing, no text, no watermark."
            ),
            Style::Painted => "Style: a painted illustration with clear shapes and flat areas \
                               of colour, no text, no watermark."
                .to_string(),
            Style::None => String::new(),
        }
    }

    /// The text sent to the generator: the description (trimmed), a blank line and the
    /// style's sentence.
    pub fn prompt(self, description: &str, colors: u16) -> String {
        let description = description.trim();
        match self {
            Style::None => description.to_string(),
            _ => format!("{description}\n\n{}", self.suffix(colors)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_add_their_sentence() {
        assert_eq!(Style::None.prompt("  a castle  ", 32), "a castle");
        assert_eq!(
            Style::PixelArt.prompt("a castle", 32),
            "a castle\n\nStyle: classic Amiga-era pixel art, a limited palette of about 32 \
             colours, crisp hard-edged pixels, no anti-aliasing, no text, no watermark."
        );
        assert!(Style::PixelArt.prompt("x", 16).contains("about 16 colours"));
        assert!(
            Style::Painted
                .prompt("x", 8)
                .ends_with("flat areas of colour, no text, no watermark.")
        );
        for s in [Style::PixelArt, Style::Painted, Style::None] {
            assert!(s.suffix(256).is_ascii());
        }
        assert_eq!(Style::default(), Style::PixelArt);
    }
}
