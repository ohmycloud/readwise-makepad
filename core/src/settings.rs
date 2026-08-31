#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AppLanguage {
    #[default]
    Chinese,
    English,
}

impl AppLanguage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chinese => "zh-CN",
            Self::English => "en",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "en" | "en-US" | "en-GB" => Self::English,
            _ => Self::Chinese,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LibraryViewMode {
    #[default]
    List,
    Grid,
}

impl LibraryViewMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Grid => "grid",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "grid" => Self::Grid,
            _ => Self::List,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReaderTheme {
    #[default]
    Dark,
    Light,
}

impl ReaderTheme {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "light" => Self::Light,
            _ => Self::Dark,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReaderBackground {
    FollowTheme,
    #[default]
    Paper,
    Sepia,
    Sage,
    Night,
}

impl ReaderBackground {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FollowTheme => "theme",
            Self::Paper => "paper",
            Self::Sepia => "sepia",
            Self::Sage => "sage",
            Self::Night => "night",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "theme" => Self::FollowTheme,
            "sepia" => Self::Sepia,
            "sage" => Self::Sage,
            "night" => Self::Night,
            _ => Self::Paper,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReaderFont {
    #[default]
    Standard,
    Monospace,
}

impl ReaderFont {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Monospace => "monospace",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "monospace" => Self::Monospace,
            _ => Self::Standard,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReaderSpacing {
    #[default]
    Default,
    Comfortable,
    Relaxed,
}

impl ReaderSpacing {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Comfortable => "comfortable",
            Self::Relaxed => "relaxed",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "comfortable" => Self::Comfortable,
            "relaxed" => Self::Relaxed,
            _ => Self::Default,
        }
    }

    pub fn plain_line_spacing(self) -> f32 {
        match self {
            Self::Default => 1.2,
            Self::Comfortable => 1.45,
            Self::Relaxed => 1.75,
        }
    }

    pub fn gloss_line_spacing(self) -> f32 {
        match self {
            Self::Default => 2.2,
            Self::Comfortable => 2.45,
            Self::Relaxed => 2.75,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReaderSettings {
    pub language: AppLanguage,
    pub library_view_mode: LibraryViewMode,
    pub theme: ReaderTheme,
    pub background: ReaderBackground,
    pub font: ReaderFont,
    pub spacing: ReaderSpacing,
    pub font_size: Option<f32>,
    pub gloss_enabled: bool,
    pub gloss_font_size: f32,
    pub gloss_color_slot: usize,
    pub gloss_background_slot: usize,
    pub word_color_slot: usize,
    pub word_background_slot: usize,
    pub selection_color_slot: usize,
    pub highlight_color_slot: usize,
}

impl Default for ReaderSettings {
    fn default() -> Self {
        Self {
            language: AppLanguage::Chinese,
            library_view_mode: LibraryViewMode::List,
            theme: ReaderTheme::Dark,
            background: ReaderBackground::Paper,
            font: ReaderFont::Standard,
            spacing: ReaderSpacing::Default,
            font_size: None,
            gloss_enabled: true,
            gloss_font_size: 7.5,
            gloss_color_slot: 0,
            gloss_background_slot: 0,
            word_color_slot: 0,
            word_background_slot: 0,
            selection_color_slot: 0,
            highlight_color_slot: 0,
        }
    }
}

impl ReaderSettings {
    pub fn normalized(mut self) -> Self {
        self.font_size = self
            .font_size
            .filter(|value| value.is_finite())
            .map(|value| value.clamp(8.0, 20.0));
        if !self.gloss_font_size.is_finite() {
            self.gloss_font_size = Self::default().gloss_font_size;
        }
        self.gloss_font_size = self.gloss_font_size.clamp(6.0, 12.0);
        self.gloss_color_slot = self.gloss_color_slot.min(3);
        self.gloss_background_slot = self.gloss_background_slot.min(3);
        self.word_color_slot = self.word_color_slot.min(3);
        self.word_background_slot = self.word_background_slot.min(3);
        self.selection_color_slot = self.selection_color_slot.min(3);
        self.highlight_color_slot = self.highlight_color_slot.min(3);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_corrupt_or_out_of_range_reader_settings() {
        let settings = ReaderSettings {
            font_size: Some(100.0),
            gloss_font_size: f32::NAN,
            gloss_color_slot: 99,
            gloss_background_slot: 99,
            word_color_slot: 99,
            word_background_slot: 99,
            selection_color_slot: 99,
            highlight_color_slot: 99,
            ..ReaderSettings::default()
        }
        .normalized();

        assert_eq!(settings.font_size, Some(20.0));
        assert_eq!(settings.gloss_font_size, 7.5);
        assert_eq!(settings.highlight_color_slot, 3);
    }

    #[test]
    fn reader_theme_uses_stable_storage_values() {
        assert_eq!(ReaderTheme::Dark.as_str(), "dark");
        assert_eq!(ReaderTheme::Light.as_str(), "light");
        assert_eq!(ReaderTheme::from_str("light"), ReaderTheme::Light);
        assert_eq!(ReaderTheme::from_str("unknown"), ReaderTheme::Dark);
    }

    #[test]
    fn reader_background_uses_stable_storage_values() {
        for background in [
            ReaderBackground::FollowTheme,
            ReaderBackground::Paper,
            ReaderBackground::Sepia,
            ReaderBackground::Sage,
            ReaderBackground::Night,
        ] {
            assert_eq!(ReaderBackground::from_str(background.as_str()), background);
        }
        assert_eq!(
            ReaderBackground::from_str("unknown"),
            ReaderBackground::Paper
        );
    }

    #[test]
    fn app_language_uses_stable_storage_values() {
        assert_eq!(AppLanguage::Chinese.as_str(), "zh-CN");
        assert_eq!(AppLanguage::English.as_str(), "en");
        assert_eq!(AppLanguage::from_str("en-US"), AppLanguage::English);
        assert_eq!(AppLanguage::from_str("unknown"), AppLanguage::Chinese);
    }

    #[test]
    fn library_view_mode_uses_stable_storage_values() {
        assert_eq!(LibraryViewMode::List.as_str(), "list");
        assert_eq!(LibraryViewMode::Grid.as_str(), "grid");
        assert_eq!(LibraryViewMode::from_str("grid"), LibraryViewMode::Grid);
        assert_eq!(LibraryViewMode::from_str("unknown"), LibraryViewMode::List);
    }

    #[test]
    fn reader_font_uses_stable_storage_values() {
        assert_eq!(ReaderFont::Standard.as_str(), "standard");
        assert_eq!(ReaderFont::Monospace.as_str(), "monospace");
        assert_eq!(ReaderFont::from_str("monospace"), ReaderFont::Monospace);
        assert_eq!(ReaderFont::from_str("unknown"), ReaderFont::Standard);
    }

    #[test]
    fn reader_spacing_preserves_extra_room_for_glosses() {
        for spacing in [
            ReaderSpacing::Default,
            ReaderSpacing::Comfortable,
            ReaderSpacing::Relaxed,
        ] {
            assert!(spacing.gloss_line_spacing() > spacing.plain_line_spacing());
            assert!(spacing.gloss_line_spacing() >= 2.2);
            assert_eq!(ReaderSpacing::from_str(spacing.as_str()), spacing);
        }
        assert_eq!(ReaderSpacing::from_str("unknown"), ReaderSpacing::Default);
    }
}
