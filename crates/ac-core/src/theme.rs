//! Theme system for TUI color palettes and styling.
//!
//! Defines color mappings for UI elements (borders, text, backgrounds, etc.)
//! to support multiple visual themes (Default, Dark, High-Contrast, Light, etc.).

use serde::{Deserialize, Serialize};

/// Color palette for the TUI interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub border: Color,
    pub border_focused: Color,
    pub title: Color,
    pub title_focused: Color,
    pub text: Color,
    pub text_dim: Color,
    pub text_highlight: Color,
    pub text_muted: Color,
    pub accent: Color,
    pub accent_secondary: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub info: Color,
    pub background: Option<Color>,
    pub selected_bg: Option<Color>,
    pub header_bg: Option<Color>,
    pub footer_bg: Option<Color>,
}

/// Color representation compatible with ratatui.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Color {
    Named(NamedColor),
    Rgb { r: u8, g: u8, b: u8 },
}

/// Named colors that map to ratatui's Color enum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NamedColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    LightBlack,
    LightRed,
    LightGreen,
    LightYellow,
    LightBlue,
    LightMagenta,
    LightCyan,
    LightWhite,
    DarkGray,
    Gray,
    Purple,
}

impl Theme {
    /// Get the Default theme (enhanced cyan/blue palette).
    pub fn default_theme() -> Self {
        Self {
            name: "Default".to_string(),
            border: Color::Rgb {
                r: 100,
                g: 149,
                b: 237,
            },
            border_focused: Color::Rgb {
                r: 56,
                g: 189,
                b: 248,
            },
            title: Color::Rgb {
                r: 56,
                g: 189,
                b: 248,
            },
            title_focused: Color::Rgb {
                r: 56,
                g: 189,
                b: 248,
            },
            text: NamedColor::White.into(),
            text_dim: Color::Rgb {
                r: 148,
                g: 163,
                b: 184,
            },
            text_highlight: Color::Rgb {
                r: 255,
                g: 245,
                b: 157,
            },
            text_muted: Color::Rgb {
                r: 148,
                g: 163,
                b: 184,
            },
            accent: Color::Rgb {
                r: 56,
                g: 189,
                b: 248,
            },
            accent_secondary: Color::Rgb {
                r: 129,
                g: 140,
                b: 248,
            },
            success: Color::Rgb {
                r: 102,
                g: 187,
                b: 106,
            },
            warning: Color::Rgb {
                r: 255,
                g: 193,
                b: 7,
            },
            error: Color::Rgb {
                r: 239,
                g: 83,
                b: 80,
            },
            info: Color::Rgb {
                r: 79,
                g: 195,
                b: 247,
            },
            background: None,
            selected_bg: Some(Color::Rgb {
                r: 50,
                g: 60,
                b: 80,
            }),
            header_bg: None,
            footer_bg: None,
        }
    }

    /// Get the Dark theme (enhanced with better contrast).
    pub fn dark_theme() -> Self {
        Self {
            name: "Dark".to_string(),
            border: Color::Rgb {
                r: 80,
                g: 90,
                b: 100,
            },
            border_focused: Color::Rgb {
                r: 100,
                g: 149,
                b: 237,
            },
            title: Color::Rgb {
                r: 100,
                g: 149,
                b: 237,
            },
            title_focused: Color::Rgb {
                r: 100,
                g: 149,
                b: 237,
            },
            text: Color::Rgb {
                r: 230,
                g: 230,
                b: 230,
            },
            text_dim: Color::Rgb {
                r: 120,
                g: 130,
                b: 140,
            },
            text_highlight: Color::Rgb {
                r: 255,
                g: 204,
                b: 102,
            },
            text_muted: Color::Rgb {
                r: 120,
                g: 130,
                b: 140,
            },
            accent: Color::Rgb {
                r: 100,
                g: 149,
                b: 237,
            },
            accent_secondary: Color::Rgb {
                r: 67,
                g: 133,
                b: 240,
            },
            success: Color::Rgb {
                r: 46,
                g: 204,
                b: 113,
            },
            warning: Color::Rgb {
                r: 255,
                g: 193,
                b: 7,
            },
            error: Color::Rgb {
                r: 239,
                g: 68,
                b: 68,
            },
            info: Color::Rgb {
                r: 79,
                g: 195,
                b: 247,
            },
            background: None,
            selected_bg: Some(Color::Rgb {
                r: 40,
                g: 45,
                b: 55,
            }),
            header_bg: None,
            footer_bg: None,
        }
    }

    /// Get the High-Contrast theme (maximal contrast for accessibility).
    pub fn high_contrast_theme() -> Self {
        Self {
            name: "High-Contrast".to_string(),
            border: NamedColor::White.into(),
            border_focused: Color::Rgb {
                r: 255,
                g: 255,
                b: 0,
            },
            title: Color::Rgb {
                r: 255,
                g: 255,
                b: 0,
            },
            title_focused: Color::Rgb {
                r: 255,
                g: 255,
                b: 0,
            },
            text: NamedColor::White.into(),
            text_dim: Color::Rgb {
                r: 200,
                g: 200,
                b: 200,
            },
            text_highlight: Color::Rgb {
                r: 255,
                g: 255,
                b: 0,
            },
            text_muted: Color::Rgb {
                r: 200,
                g: 200,
                b: 200,
            },
            accent: Color::Rgb {
                r: 255,
                g: 255,
                b: 0,
            },
            accent_secondary: Color::Rgb {
                r: 0,
                g: 255,
                b: 255,
            },
            success: Color::Rgb { r: 0, g: 255, b: 0 },
            warning: Color::Rgb {
                r: 255,
                g: 255,
                b: 0,
            },
            error: Color::Rgb { r: 255, g: 0, b: 0 },
            info: Color::Rgb {
                r: 0,
                g: 255,
                b: 255,
            },
            background: Some(NamedColor::Black.into()),
            selected_bg: Some(Color::Rgb {
                r: 100,
                g: 100,
                b: 100,
            }),
            header_bg: Some(Color::Rgb {
                r: 30,
                g: 30,
                b: 30,
            }),
            footer_bg: Some(Color::Rgb {
                r: 30,
                g: 30,
                b: 30,
            }),
        }
    }

    /// Get the Light theme (light background with dark text, improved).
    pub fn light_theme() -> Self {
        Self {
            name: "Light".to_string(),
            border: Color::Rgb {
                r: 150,
                g: 150,
                b: 150,
            },
            border_focused: Color::Rgb {
                r: 59,
                g: 130,
                b: 246,
            },
            title: Color::Rgb {
                r: 59,
                g: 130,
                b: 246,
            },
            title_focused: Color::Rgb {
                r: 59,
                g: 130,
                b: 246,
            },
            text: NamedColor::Black.into(),
            text_dim: Color::Rgb {
                r: 100,
                g: 100,
                b: 100,
            },
            text_highlight: Color::Rgb {
                r: 217,
                g: 70,
                b: 239,
            },
            text_muted: Color::Rgb {
                r: 120,
                g: 120,
                b: 120,
            },
            accent: Color::Rgb {
                r: 59,
                g: 130,
                b: 246,
            },
            accent_secondary: Color::Rgb {
                r: 16,
                g: 185,
                b: 129,
            },
            success: Color::Rgb {
                r: 16,
                g: 185,
                b: 129,
            },
            warning: Color::Rgb {
                r: 245,
                g: 158,
                b: 11,
            },
            error: Color::Rgb {
                r: 220,
                g: 53,
                b: 69,
            },
            info: Color::Rgb {
                r: 59,
                g: 130,
                b: 246,
            },
            background: Some(NamedColor::White.into()),
            selected_bg: Some(Color::Rgb {
                r: 220,
                g: 230,
                b: 240,
            }),
            header_bg: Some(Color::Rgb {
                r: 245,
                g: 245,
                b: 245,
            }),
            footer_bg: Some(Color::Rgb {
                r: 245,
                g: 245,
                b: 245,
            }),
        }
    }

    /// Get the Forest theme (green/nature inspired, vibrant).
    pub fn forest_theme() -> Self {
        Self {
            name: "Forest".to_string(),
            border: Color::Rgb {
                r: 34,
                g: 139,
                b: 34,
            },
            border_focused: Color::Rgb {
                r: 50,
                g: 205,
                b: 50,
            },
            title: Color::Rgb {
                r: 50,
                g: 205,
                b: 50,
            },
            title_focused: Color::Rgb {
                r: 50,
                g: 205,
                b: 50,
            },
            text: Color::Rgb {
                r: 220,
                g: 250,
                b: 220,
            },
            text_dim: Color::Rgb {
                r: 100,
                g: 140,
                b: 100,
            },
            text_highlight: Color::Rgb {
                r: 154,
                g: 205,
                b: 50,
            },
            text_muted: Color::Rgb {
                r: 100,
                g: 140,
                b: 100,
            },
            accent: Color::Rgb {
                r: 34,
                g: 139,
                b: 34,
            },
            accent_secondary: Color::Rgb {
                r: 76,
                g: 175,
                b: 80,
            },
            success: Color::Rgb {
                r: 76,
                g: 175,
                b: 80,
            },
            warning: Color::Rgb {
                r: 255,
                g: 193,
                b: 7,
            },
            error: Color::Rgb {
                r: 220,
                g: 53,
                b: 69,
            },
            info: Color::Rgb {
                r: 52,
                g: 152,
                b: 219,
            },
            background: None,
            selected_bg: Some(Color::Rgb {
                r: 30,
                g: 60,
                b: 40,
            }),
            header_bg: None,
            footer_bg: None,
        }
    }

    /// Get the Ocean theme (blue/sea inspired, vibrant).
    pub fn ocean_theme() -> Self {
        Self {
            name: "Ocean".to_string(),
            border: Color::Rgb {
                r: 65,
                g: 105,
                b: 225,
            },
            border_focused: Color::Rgb {
                r: 100,
                g: 149,
                b: 237,
            },
            title: Color::Rgb {
                r: 100,
                g: 149,
                b: 237,
            },
            title_focused: Color::Rgb {
                r: 100,
                g: 149,
                b: 237,
            },
            text: Color::Rgb {
                r: 220,
                g: 240,
                b: 255,
            },
            text_dim: Color::Rgb {
                r: 100,
                g: 130,
                b: 160,
            },
            text_highlight: Color::Rgb {
                r: 255,
                g: 255,
                b: 255,
            },
            text_muted: Color::Rgb {
                r: 100,
                g: 130,
                b: 160,
            },
            accent: Color::Rgb {
                r: 65,
                g: 105,
                b: 225,
            },
            accent_secondary: Color::Rgb {
                r: 135,
                g: 206,
                b: 250,
            },
            success: Color::Rgb {
                r: 135,
                g: 206,
                b: 250,
            },
            warning: Color::Rgb {
                r: 255,
                g: 215,
                b: 0,
            },
            error: Color::Rgb {
                r: 255,
                g: 99,
                b: 71,
            },
            info: Color::Rgb {
                r: 79,
                g: 195,
                b: 247,
            },
            background: None,
            selected_bg: Some(Color::Rgb {
                r: 30,
                g: 50,
                b: 70,
            }),
            header_bg: None,
            footer_bg: None,
        }
    }

    /// Get the Dracula theme (popular dark theme, vibrant).
    pub fn dracula_theme() -> Self {
        Self {
            name: "Dracula".to_string(),
            border: Color::Rgb {
                r: 189,
                g: 147,
                b: 249,
            },
            border_focused: Color::Rgb {
                r: 255,
                g: 121,
                b: 198,
            },
            title: Color::Rgb {
                r: 255,
                g: 121,
                b: 198,
            },
            title_focused: Color::Rgb {
                r: 255,
                g: 121,
                b: 198,
            },
            text: Color::Rgb {
                r: 248,
                g: 248,
                b: 248,
            },
            text_dim: Color::Rgb {
                r: 98,
                g: 114,
                b: 164,
            },
            text_highlight: Color::Rgb {
                r: 255,
                g: 184,
                b: 108,
            },
            text_muted: Color::Rgb {
                r: 98,
                g: 114,
                b: 164,
            },
            accent: Color::Rgb {
                r: 189,
                g: 147,
                b: 249,
            },
            accent_secondary: Color::Rgb {
                r: 139,
                g: 233,
                b: 253,
            },
            success: Color::Rgb {
                r: 80,
                g: 250,
                b: 123,
            },
            warning: Color::Rgb {
                r: 241,
                g: 250,
                b: 140,
            },
            error: Color::Rgb {
                r: 255,
                g: 85,
                b: 85,
            },
            info: Color::Rgb {
                r: 98,
                g: 114,
                b: 239,
            },
            background: None,
            selected_bg: Some(Color::Rgb {
                r: 68,
                g: 71,
                b: 90,
            }),
            header_bg: None,
            footer_bg: None,
        }
    }

    /// Get the Nord theme (cool arctic theme, vibrant).
    pub fn nord_theme() -> Self {
        Self {
            name: "Nord".to_string(),
            border: Color::Rgb {
                r: 94,
                g: 129,
                b: 172,
            },
            border_focused: Color::Rgb {
                r: 136,
                g: 192,
                b: 208,
            },
            title: Color::Rgb {
                r: 136,
                g: 192,
                b: 208,
            },
            title_focused: Color::Rgb {
                r: 136,
                g: 192,
                b: 208,
            },
            text: Color::Rgb {
                r: 229,
                g: 233,
                b: 240,
            },
            text_dim: Color::Rgb {
                r: 94,
                g: 129,
                b: 172,
            },
            text_highlight: Color::Rgb {
                r: 235,
                g: 203,
                b: 139,
            },
            text_muted: Color::Rgb {
                r: 94,
                g: 129,
                b: 172,
            },
            accent: Color::Rgb {
                r: 94,
                g: 129,
                b: 172,
            },
            accent_secondary: Color::Rgb {
                r: 129,
                g: 161,
                b: 193,
            },
            success: Color::Rgb {
                r: 163,
                g: 190,
                b: 140,
            },
            warning: Color::Rgb {
                r: 235,
                g: 203,
                b: 139,
            },
            error: Color::Rgb {
                r: 191,
                g: 97,
                b: 106,
            },
            info: Color::Rgb {
                r: 129,
                g: 161,
                b: 193,
            },
            background: None,
            selected_bg: Some(Color::Rgb {
                r: 59,
                g: 66,
                b: 82,
            }),
            header_bg: None,
            footer_bg: None,
        }
    }

    /// Get the Solarized Light theme (warm, vibrant).
    pub fn solarized_light_theme() -> Self {
        Self {
            name: "Solarized Light".to_string(),
            border: Color::Rgb {
                r: 181,
                g: 137,
                b: 0,
            },
            border_focused: Color::Rgb {
                r: 38,
                g: 139,
                b: 210,
            },
            title: Color::Rgb {
                r: 38,
                g: 139,
                b: 210,
            },
            title_focused: Color::Rgb {
                r: 38,
                g: 139,
                b: 210,
            },
            text: Color::Rgb {
                r: 43,
                g: 43,
                b: 43,
            },
            text_dim: Color::Rgb {
                r: 147,
                g: 161,
                b: 161,
            },
            text_highlight: Color::Rgb {
                r: 203,
                g: 75,
                b: 22,
            },
            text_muted: Color::Rgb {
                r: 147,
                g: 161,
                b: 161,
            },
            accent: Color::Rgb {
                r: 38,
                g: 139,
                b: 210,
            },
            accent_secondary: Color::Rgb {
                r: 133,
                g: 153,
                b: 0,
            },
            success: Color::Rgb {
                r: 133,
                g: 153,
                b: 0,
            },
            warning: Color::Rgb {
                r: 181,
                g: 137,
                b: 0,
            },
            error: Color::Rgb {
                r: 220,
                g: 50,
                b: 47,
            },
            info: Color::Rgb {
                r: 38,
                g: 139,
                b: 210,
            },
            background: Some(NamedColor::White.into()),
            selected_bg: Some(Color::Rgb {
                r: 238,
                g: 232,
                b: 213,
            }),
            header_bg: Some(Color::Rgb {
                r: 253,
                g: 246,
                b: 227,
            }),
            footer_bg: Some(Color::Rgb {
                r: 253,
                g: 246,
                b: 227,
            }),
        }
    }

    /// Get the Gruvbox theme (warm retro theme, vibrant).
    pub fn gruvbox_theme() -> Self {
        Self {
            name: "Gruvbox".to_string(),
            border: Color::Rgb {
                r: 215,
                g: 95,
                b: 6,
            },
            border_focused: Color::Rgb {
                r: 251,
                g: 121,
                b: 6,
            },
            title: Color::Rgb {
                r: 251,
                g: 121,
                b: 6,
            },
            title_focused: Color::Rgb {
                r: 251,
                g: 121,
                b: 6,
            },
            text: Color::Rgb {
                r: 251,
                g: 241,
                b: 199,
            },
            text_dim: Color::Rgb {
                r: 146,
                g: 130,
                b: 109,
            },
            text_highlight: Color::Rgb {
                r: 254,
                g: 208,
                b: 68,
            },
            text_muted: Color::Rgb {
                r: 146,
                g: 130,
                b: 109,
            },
            accent: Color::Rgb {
                r: 215,
                g: 95,
                b: 6,
            },
            accent_secondary: Color::Rgb {
                r: 214,
                g: 93,
                b: 5,
            },
            success: Color::Rgb {
                r: 152,
                g: 195,
                b: 121,
            },
            warning: Color::Rgb {
                r: 254,
                g: 208,
                b: 68,
            },
            error: Color::Rgb {
                r: 251,
                g: 73,
                b: 48,
            },
            info: Color::Rgb {
                r: 104,
                g: 157,
                b: 106,
            },
            background: None,
            selected_bg: Some(Color::Rgb {
                r: 60,
                g: 56,
                b: 54,
            }),
            header_bg: None,
            footer_bg: None,
        }
    }

    /// Get the Tokyo Night theme (modern dark blue theme, vibrant).
    pub fn tokyo_night_theme() -> Self {
        Self {
            name: "Tokyo Night".to_string(),
            border: Color::Rgb {
                r: 122,
                g: 162,
                b: 247,
            },
            border_focused: Color::Rgb {
                r: 187,
                g: 154,
                b: 247,
            },
            title: Color::Rgb {
                r: 187,
                g: 154,
                b: 247,
            },
            title_focused: Color::Rgb {
                r: 187,
                g: 154,
                b: 247,
            },
            text: Color::Rgb {
                r: 248,
                g: 248,
                b: 248,
            },
            text_dim: Color::Rgb {
                r: 113,
                g: 119,
                b: 138,
            },
            text_highlight: Color::Rgb {
                r: 221,
                g: 182,
                b: 242,
            },
            text_muted: Color::Rgb {
                r: 113,
                g: 119,
                b: 138,
            },
            accent: Color::Rgb {
                r: 122,
                g: 162,
                b: 247,
            },
            accent_secondary: Color::Rgb {
                r: 207,
                g: 207,
                b: 254,
            },
            success: Color::Rgb {
                r: 78,
                g: 201,
                b: 176,
            },
            warning: Color::Rgb {
                r: 249,
                g: 224,
                b: 224,
            },
            error: Color::Rgb {
                r: 242,
                g: 139,
                b: 130,
            },
            info: Color::Rgb {
                r: 207,
                g: 207,
                b: 254,
            },
            background: None,
            selected_bg: Some(Color::Rgb {
                r: 41,
                g: 59,
                b: 85,
            }),
            header_bg: None,
            footer_bg: None,
        }
    }

    /// Get the Monokai theme (classic editor theme, vibrant).
    pub fn monokai_theme() -> Self {
        Self {
            name: "Monokai".to_string(),
            border: Color::Rgb {
                r: 255,
                g: 95,
                b: 135,
            },
            border_focused: Color::Rgb {
                r: 255,
                g: 146,
                b: 160,
            },
            title: Color::Rgb {
                r: 255,
                g: 146,
                b: 160,
            },
            title_focused: Color::Rgb {
                r: 255,
                g: 146,
                b: 160,
            },
            text: Color::Rgb {
                r: 248,
                g: 248,
                b: 242,
            },
            text_dim: Color::Rgb {
                r: 80,
                g: 80,
                b: 80,
            },
            text_highlight: Color::Rgb {
                r: 255,
                g: 235,
                b: 59,
            },
            text_muted: Color::Rgb {
                r: 80,
                g: 80,
                b: 80,
            },
            accent: Color::Rgb {
                r: 255,
                g: 95,
                b: 135,
            },
            accent_secondary: Color::Rgb {
                r: 106,
                g: 190,
                b: 48,
            },
            success: Color::Rgb {
                r: 106,
                g: 190,
                b: 48,
            },
            warning: Color::Rgb {
                r: 255,
                g: 235,
                b: 59,
            },
            error: Color::Rgb {
                r: 255,
                g: 80,
                b: 80,
            },
            info: Color::Rgb {
                r: 174,
                g: 129,
                b: 255,
            },
            background: None,
            selected_bg: Some(Color::Rgb {
                r: 59,
                g: 58,
                b: 66,
            }),
            header_bg: None,
            footer_bg: None,
        }
    }

    /// Get the Cyberpunk theme (neon vibrant colors - Cyberpunk 2077 style).
    pub fn cyberpunk_theme() -> Self {
        Self {
            name: "Cyberpunk".to_string(),
            border: Color::Rgb {
                r: 255,
                g: 0,
                b: 128,
            }, // Hot pink neon
            border_focused: Color::Rgb {
                r: 255,
                g: 105,
                b: 180,
            }, // Bright neon pink
            title: Color::Rgb {
                r: 255,
                g: 0,
                b: 128,
            }, // Hot pink
            title_focused: Color::Rgb {
                r: 0,
                g: 255,
                b: 255,
            }, // Cyan neon
            text: NamedColor::White.into(),
            text_dim: Color::Rgb {
                r: 180,
                g: 160,
                b: 255,
            }, // Purple gray
            text_highlight: Color::Rgb {
                r: 255,
                g: 255,
                b: 0,
            }, // Yellow neon
            text_muted: Color::Rgb {
                r: 140,
                g: 100,
                b: 200,
            }, // Muted purple
            accent: Color::Rgb {
                r: 255,
                g: 0,
                b: 128,
            }, // Hot pink
            accent_secondary: Color::Rgb {
                r: 0,
                g: 255,
                b: 255,
            }, // Cyan neon
            success: Color::Rgb {
                r: 57,
                g: 255,
                b: 20,
            }, // Bright green neon
            warning: Color::Rgb {
                r: 255,
                g: 255,
                b: 0,
            }, // Yellow neon
            error: Color::Rgb {
                r: 255,
                g: 50,
                b: 50,
            }, // Bright red neon
            info: Color::Rgb {
                r: 0,
                g: 200,
                b: 255,
            }, // Blue neon
            background: Some(Color::Rgb {
                r: 20,
                g: 10,
                b: 30,
            }), // Dark purple-black
            selected_bg: Some(Color::Rgb {
                r: 60,
                g: 20,
                b: 80,
            }), // Purple highlight
            header_bg: Some(Color::Rgb {
                r: 25,
                g: 15,
                b: 35,
            }),
            footer_bg: Some(Color::Rgb {
                r: 25,
                g: 15,
                b: 35,
            }),
        }
    }

    /// Get all available theme presets.
    pub fn all_presets() -> Vec<Self> {
        vec![
            Self::default_theme(),
            Self::dark_theme(),
            Self::high_contrast_theme(),
            Self::light_theme(),
            Self::forest_theme(),
            Self::ocean_theme(),
            Self::dracula_theme(),
            Self::nord_theme(),
            Self::solarized_light_theme(),
            Self::gruvbox_theme(),
            Self::tokyo_night_theme(),
            Self::monokai_theme(),
            Self::cyberpunk_theme(),
        ]
    }

    /// Get a theme by name (case-insensitive).
    pub fn by_name(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "default" => Some(Self::default_theme()),
            "dark" => Some(Self::dark_theme()),
            "high-contrast" | "high contrast" => Some(Self::high_contrast_theme()),
            "light" => Some(Self::light_theme()),
            "forest" => Some(Self::forest_theme()),
            "ocean" => Some(Self::ocean_theme()),
            "dracula" => Some(Self::dracula_theme()),
            "nord" => Some(Self::nord_theme()),
            "solarized" | "solarized light" => Some(Self::solarized_light_theme()),
            "gruvbox" => Some(Self::gruvbox_theme()),
            "tokyo" | "tokyo night" => Some(Self::tokyo_night_theme()),
            "monokai" => Some(Self::monokai_theme()),
            "cyberpunk" => Some(Self::cyberpunk_theme()),
            _ => None,
        }
    }
}

impl From<NamedColor> for Color {
    fn from(named: NamedColor) -> Self {
        Color::Named(named)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::default_theme()
    }
}
