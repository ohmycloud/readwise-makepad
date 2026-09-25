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
    Original,
    #[default]
    Paper,
    Sepia,
    Sage,
    Bold,
    Night,
}

impl ReaderBackground {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FollowTheme => "theme",
            Self::Original => "original",
            Self::Paper => "paper",
            Self::Sepia => "sepia",
            Self::Sage => "sage",
            Self::Bold => "bold",
            Self::Night => "night",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "theme" => Self::FollowTheme,
            "original" => Self::Original,
            "sepia" => Self::Sepia,
            "sage" => Self::Sage,
            "bold" => Self::Bold,
            "night" => Self::Night,
            _ => Self::Paper,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReaderTurnMode {
    #[default]
    Curl,
    Slide,
    Scroll,
}

impl ReaderTurnMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Curl => "curl",
            Self::Slide => "slide",
            Self::Scroll => "scroll",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "slide" => Self::Slide,
            "scroll" => Self::Scroll,
            _ => Self::Curl,
        }
    }

    pub fn is_paged(self) -> bool {
        self != Self::Scroll
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReaderColumns {
    One,
    Two,
    #[default]
    Auto,
}

impl ReaderColumns {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::One => "one",
            Self::Two => "two",
            Self::Auto => "auto",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "one" => Self::One,
            "two" => Self::Two,
            _ => Self::Auto,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReaderFont {
    Canela,
    #[default]
    Original,
    Athelas,
    AvenirNext,
    Charter,
    Georgia,
    Iowan,
    Palatino,
    ProximaNova,
    Publico,
    SanFrancisco,
    NewYork,
    Seravek,
    TimesNewRoman,
}

impl ReaderFont {
    pub const ALL: [Self; 14] = [
        Self::Canela,
        Self::Original,
        Self::Athelas,
        Self::AvenirNext,
        Self::Charter,
        Self::Georgia,
        Self::Iowan,
        Self::Palatino,
        Self::ProximaNova,
        Self::Publico,
        Self::SanFrancisco,
        Self::NewYork,
        Self::Seravek,
        Self::TimesNewRoman,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Canela => "Canela",
            Self::Original => "Original",
            Self::Athelas => "Athelas",
            Self::AvenirNext => "Avenir Next",
            Self::Charter => "Charter",
            Self::Georgia => "Georgia",
            Self::Iowan => "Iowan",
            Self::Palatino => "Palatino",
            Self::ProximaNova => "Proxima Nova",
            Self::Publico => "Publico",
            Self::SanFrancisco => "San Francisco",
            Self::NewYork => "New York",
            Self::Seravek => "Seravek",
            Self::TimesNewRoman => "Times New Roman",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|font| *font == self).unwrap()
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Canela => "canela",
            Self::Original => "original",
            Self::Athelas => "athelas",
            Self::AvenirNext => "avenir_next",
            Self::Charter => "charter",
            Self::Georgia => "georgia",
            Self::Iowan => "iowan",
            Self::Palatino => "palatino",
            Self::ProximaNova => "proxima_nova",
            Self::Publico => "publico",
            Self::SanFrancisco => "san_francisco",
            Self::NewYork => "new_york",
            Self::Seravek => "seravek",
            Self::TimesNewRoman => "times_new_roman",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "canela" => Self::Canela,
            "athelas" => Self::Athelas,
            "avenir_next" => Self::AvenirNext,
            "charter" => Self::Charter,
            "georgia" => Self::Georgia,
            "iowan" => Self::Iowan,
            "palatino" => Self::Palatino,
            "proxima_nova" => Self::ProximaNova,
            "publico" => Self::Publico,
            "san_francisco" => Self::SanFrancisco,
            "new_york" => Self::NewYork,
            "seravek" => Self::Seravek,
            "times_new_roman" => Self::TimesNewRoman,
            _ => Self::Original,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ReaderSpacing {
    #[default]
    Default,
    Comfortable,
    Relaxed,
    Custom(f32),
}

impl ReaderSpacing {
    pub const MIN: f32 = 0.8;
    pub const MAX: f32 = 2.5;

    pub fn as_str(self) -> String {
        match self {
            Self::Default => "default".to_string(),
            Self::Comfortable => "1.85".to_string(),
            Self::Relaxed => "2.15".to_string(),
            Self::Custom(value) => value.to_string(),
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "comfortable" => Self::Custom(1.45),
            "relaxed" => Self::Custom(1.75),
            "default" => Self::Default,
            _ => value.parse().map(Self::from_value).unwrap_or_default(),
        }
    }

    pub fn from_value(value: f32) -> Self {
        if !value.is_finite() {
            return Self::Default;
        }
        let value = value.clamp(Self::MIN, Self::MAX);
        if (value - 1.55).abs() < 0.005 {
            Self::Default
        } else if (value - 1.85).abs() < 0.005 {
            Self::Comfortable
        } else if (value - 2.15).abs() < 0.005 {
            Self::Relaxed
        } else {
            Self::Custom(value)
        }
    }

    pub fn plain_line_spacing(self) -> f32 {
        match self {
            Self::Default => 1.55,
            Self::Comfortable => 1.85,
            Self::Relaxed => 2.15,
            Self::Custom(value) => value,
        }
    }

    pub fn gloss_line_spacing(self) -> f32 {
        self.plain_line_spacing() + 1.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReaderSettings {
    pub language: AppLanguage,
    pub library_view_mode: LibraryViewMode,
    pub theme: ReaderTheme,
    pub background: ReaderBackground,
    pub columns: ReaderColumns,
    pub turn_mode: ReaderTurnMode,
    pub font: ReaderFont,
    pub bold_text: bool,
    pub spacing: ReaderSpacing,
    pub character_spacing: f32,
    pub word_spacing: f32,
    /// Percentage adjustment to the default reader margin, from -10 to 10.
    pub margin: f32,
    pub justify_text: bool,
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
            columns: ReaderColumns::Auto,
            turn_mode: ReaderTurnMode::Curl,
            font: ReaderFont::Original,
            bold_text: false,
            spacing: ReaderSpacing::Default,
            character_spacing: 0.0,
            word_spacing: 0.0,
            margin: 0.0,
            justify_text: false,
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
        self.spacing = ReaderSpacing::from_value(self.spacing.plain_line_spacing());
        self.font_size = self
            .font_size
            .filter(|value| value.is_finite())
            .map(|value| value.clamp(8.0, 20.0));
        if !self.character_spacing.is_finite() {
            self.character_spacing = 0.0;
        }
        self.character_spacing = self.character_spacing.clamp(-0.1, 0.1);
        if !self.word_spacing.is_finite() {
            self.word_spacing = 0.0;
        }
        self.word_spacing = self.word_spacing.clamp(-0.2, 0.2);
        if !self.margin.is_finite() {
            self.margin = Self::default().margin;
        }
        self.margin = self.margin.clamp(-10.0, 10.0);
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

    pub fn margin_pixels(margin_percent: f32) -> f32 {
        28.0 * (1.0 + margin_percent.clamp(-10.0, 10.0) / 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_columns_round_trip_and_unknown_defaults_to_auto() {
        for mode in [ReaderColumns::One, ReaderColumns::Two, ReaderColumns::Auto] {
            assert_eq!(ReaderColumns::from_str(mode.as_str()), mode);
        }
        assert_eq!(ReaderColumns::from_str("unknown"), ReaderColumns::Auto);
        assert_eq!(ReaderSettings::default().columns, ReaderColumns::Auto);
    }

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
        assert_eq!(
            ReaderFont::ALL.map(ReaderFont::label),
            [
                "Canela",
                "Original",
                "Athelas",
                "Avenir Next",
                "Charter",
                "Georgia",
                "Iowan",
                "Palatino",
                "Proxima Nova",
                "Publico",
                "San Francisco",
                "New York",
                "Seravek",
                "Times New Roman",
            ]
        );
        for (index, font) in ReaderFont::ALL.into_iter().enumerate() {
            assert_eq!(ReaderFont::from_str(font.as_str()), font);
            assert_eq!(ReaderFont::from_index(index), Some(font));
            assert!(!font.label().is_empty());
        }
        assert_eq!(ReaderFont::from_str("standard"), ReaderFont::Original);
        assert_eq!(ReaderFont::from_str("monospace"), ReaderFont::Original);
        assert_eq!(ReaderFont::from_str("unknown"), ReaderFont::Original);
    }

    #[test]
    fn reader_spacing_preserves_extra_room_for_glosses() {
        for spacing in [
            ReaderSpacing::Default,
            ReaderSpacing::Comfortable,
            ReaderSpacing::Relaxed,
            ReaderSpacing::Custom(0.8),
            ReaderSpacing::Custom(2.5),
        ] {
            assert!(spacing.gloss_line_spacing() > spacing.plain_line_spacing());
            assert_eq!(ReaderSpacing::from_str(&spacing.as_str()), spacing);
        }
        assert_eq!(ReaderSpacing::from_str("unknown"), ReaderSpacing::Default);
        assert_eq!(
            ReaderSpacing::from_str("comfortable").plain_line_spacing(),
            1.45
        );
        assert_eq!(
            ReaderSpacing::from_str("relaxed").plain_line_spacing(),
            1.75
        );
        assert_eq!(ReaderSpacing::Default.plain_line_spacing(), 1.55);
        assert_eq!(ReaderSpacing::from_value(9.0).plain_line_spacing(), 2.5);
    }

    #[test]
    fn reader_layout_ranges_include_negative_adjustments() {
        let mut settings = ReaderSettings::default();
        assert_eq!(settings.margin, 0.0);
        assert_eq!(ReaderSettings::margin_pixels(settings.margin), 28.0);
        settings.character_spacing = -0.2;
        settings.word_spacing = 0.4;
        settings.margin = -15.0;
        let settings = settings.normalized();
        assert_eq!(settings.character_spacing, -0.1);
        assert_eq!(settings.word_spacing, 0.2);
        assert_eq!(settings.margin, -10.0);
        assert!(ReaderSettings::margin_pixels(settings.margin) < 28.0);
    }
}
