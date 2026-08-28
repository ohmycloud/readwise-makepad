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

#[derive(Clone, Debug, PartialEq)]
pub struct ReaderSettings {
    pub theme: ReaderTheme,
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
            theme: ReaderTheme::Dark,
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
}
