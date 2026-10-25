//! One validated preference contract for the global page and all three aA tabs.
use readwise_core::settings::*;
use serde::{Deserialize, Serialize};

pub const GLOSS: &[(&str, &str)] = &[
    ("青", "#7FB2A6"),
    ("蓝", "#62B6FF"),
    ("橙", "#E8875A"),
    ("灰", "#9A9A9A"),
];
pub const BACKGROUND: &[(&str, &str)] = &[
    ("无", "transparent"),
    ("奶黄", "#FFE9A8"),
    ("薄荷", "#CFE8D5"),
    ("雾蓝", "#D3E6F7"),
];
pub const WORD: &[(&str, &str)] = &[
    ("默认", "inherit"),
    ("青", "#7FB2A6"),
    ("蓝", "#62B6FF"),
    ("橙", "#E8875A"),
];
pub const PEN: &[(&str, &str)] = &[
    ("朱红", "#C1401B"),
    ("钴蓝", "#2C58C9"),
    ("苔绿", "#3B7A3D"),
    ("石墨", "#4B4A47"),
];
pub const MARKER: &[(&str, &str)] = &[
    ("黄", "#F2C438"),
    ("绿", "#7CC66F"),
    ("蓝", "#6AA8F0"),
    ("粉", "#F084AC"),
];
pub const SCENES: &[(&str, &str, &str, &str)] = &[
    ("original", "原色", "#FFFFFF", "#202020"),
    ("quiet", "静谧", "#F2E3C6", "#3B3025"),
    ("paper", "纸白", "#F7F4EA", "#2B2925"),
    ("sepia", "米黄", "#F1E6CF", "#33271B"),
    ("sage", "浅绿", "#E7EFE3", "#27342B"),
    ("snow", "雪白", "#FFFFFF", "#141518"),
    ("night", "夜间", "#20252B", "#D8D4CA"),
    ("theme", "跟随主题", "#D9D9D9", "#333333"),
];

pub const COLOUR_EXTENSION: &str = "web.annotation_colours.v1";
pub const MAX_FAVORITES: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ColourScheme {
    pub colors: [String; 4],
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ColourExtension {
    custom_colors: [Option<String>; 4],
    favorites: Vec<ColourScheme>,
}

pub fn is_hex_colour(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Preferences {
    pub language: String,
    pub theme: String,
    pub background: String,
    pub turn_mode: String,
    pub columns: String,
    pub font: String,
    pub spacing: f32,
    pub text_width: String,
    pub justify_text: bool,
    pub indent_paragraphs: bool,
    pub bold_text: bool,
    pub character_spacing: f32,
    pub word_spacing: f32,
    pub margin: f32,
    pub selection_color_slot: usize,
    pub highlight_color_slot: usize,
    pub gloss_color: usize,
    pub gloss_background: usize,
    pub word_color: usize,
    pub word_background: usize,
    pub gloss_enabled: bool,
    /// Web overrides only. Native preset slots remain 0..3.
    pub custom_colors: [Option<String>; 4],
    pub favorites: Vec<ColourScheme>,
    /// CSS pixels; native font units are half this size.
    pub font_size: f32,
    pub gloss_size: f32,
    pub english_level: String,
}

impl Default for Preferences {
    fn default() -> Self {
        Self::from(&ReaderSettings::default())
    }
}

impl From<&ReaderSettings> for Preferences {
    fn from(s: &ReaderSettings) -> Self {
        Self {
            language: s.language.as_str().into(),
            theme: s.theme.as_str().into(),
            background: s.background.as_str().into(),
            turn_mode: s.turn_mode.as_str().into(),
            columns: s.columns.as_str().into(),
            font: s.font.as_str().into(),
            spacing: s.spacing.plain_line_spacing(),
            text_width: s.text_width.as_str().into(),
            justify_text: s.justify_text,
            indent_paragraphs: s.indent_paragraphs,
            bold_text: s.bold_text,
            character_spacing: s.character_spacing,
            word_spacing: s.word_spacing,
            margin: s.margin,
            selection_color_slot: s.selection_color_slot,
            highlight_color_slot: s.highlight_color_slot,
            gloss_color: s.gloss_color_slot,
            gloss_background: s.gloss_background_slot,
            word_color: s.word_color_slot,
            word_background: s.word_background_slot,
            gloss_enabled: s.gloss_enabled,
            custom_colors: Default::default(),
            favorites: Vec::new(),
            font_size: s.font_size.unwrap_or(11.0) * 2.0,
            gloss_size: s.gloss_font_size * 1.4,
            english_level: s.english_level.as_str().into(),
        }
    }
}

impl Preferences {
    pub fn load(db: &readwise_core::Db) -> Result<Self, String> {
        let mut p = Self::from(&db.load_reader_settings()?);
        if let Some(json) = db.load_reader_extension(COLOUR_EXTENSION)? {
            let ext: ColourExtension =
                serde_json::from_str(&json).map_err(|_| "自定义配色数据无效")?;
            p.custom_colors = ext.custom_colors;
            p.favorites = ext.favorites;
        }
        p.validate()?;
        Ok(p)
    }

    pub fn save(&self, db: &readwise_core::Db) -> Result<(), String> {
        let mut native = db.load_reader_settings()?;
        self.apply(&mut native)?;
        let extension = serde_json::to_string(&ColourExtension {
            custom_colors: self.custom_colors.clone(),
            favorites: self.favorites.clone(),
        })
        .map_err(|e| e.to_string())?;
        db.save_reader_settings_with_extension(&native, Some((COLOUR_EXTENSION, &extension)))
    }

    pub fn annotation_colours(&self) -> [&str; 4] {
        let defaults = [
            GLOSS[self.gloss_color.min(3)].1,
            BACKGROUND[self.gloss_background.min(3)].1,
            WORD[self.word_color.min(3)].1,
            BACKGROUND[self.word_background.min(3)].1,
        ];
        std::array::from_fn(|index| {
            self.custom_colors[index]
                .as_deref()
                .filter(|value| is_hex_colour(value))
                .unwrap_or(defaults[index])
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        if self
            .custom_colors
            .iter()
            .flatten()
            .any(|c| !is_hex_colour(c))
            || self.favorites.len() > MAX_FAVORITES
            || self.favorites.iter().any(|scheme| {
                scheme.colors.iter().enumerate().any(|(i, c)| {
                    !is_hex_colour(c)
                        && !(matches!(i, 1 | 3) && c == "transparent")
                        && !(i == 2 && c == "inherit")
                })
            })
        {
            return Err("自定义配色无效：请使用 #RRGGBB，最多收藏 32 套".into());
        }
        let mut seen = std::collections::HashSet::new();
        for scheme in &self.favorites {
            if !seen.insert(scheme.colors.each_ref().map(|c| c.to_ascii_uppercase())) {
                return Err("这套配色已收藏".into());
            }
        }
        let in_range =
            |value: f32, low: f32, high: f32| value.is_finite() && (low..=high).contains(&value);
        if [
            self.gloss_color,
            self.gloss_background,
            self.word_color,
            self.word_background,
        ]
        .iter()
        .any(|slot| *slot > 3)
            || self.selection_color_slot > 7
            || self.highlight_color_slot > 7
            || !in_range(self.font_size, 16.0, 40.0)
            || !in_range(self.gloss_size, 8.4, 16.8)
            || !in_range(self.spacing, 0.8, 2.5)
            || !in_range(self.character_spacing, -0.1, 0.1)
            || !in_range(self.word_spacing, -0.2, 0.2)
            || !in_range(self.margin, -10.0, 10.0)
            || !["zh-CN", "en"].contains(&self.language.as_str())
            || !["light", "dark"].contains(&self.theme.as_str())
            || !SCENES.iter().any(|s| s.0 == self.background)
            || !["curl", "slide", "scroll"].contains(&self.turn_mode.as_str())
            || !["one", "two", "auto"].contains(&self.columns.as_str())
            || !["narrow", "even", "wide"].contains(&self.text_width.as_str())
            || !ReaderFont::ALL
                .iter()
                .any(|font| font.as_str() == self.font)
            || !EnglishLevel::ALL
                .iter()
                .any(|level| level.as_str() == self.english_level)
        {
            return Err("阅读设置超出允许范围".into());
        }
        Ok(())
    }

    pub fn apply(&self, s: &mut ReaderSettings) -> Result<(), String> {
        self.validate()?;
        s.language = AppLanguage::from_str(&self.language);
        s.theme = ReaderTheme::from_str(&self.theme);
        s.background = ReaderBackground::from_str(&self.background);
        s.turn_mode = ReaderTurnMode::from_str(&self.turn_mode);
        s.columns = ReaderColumns::from_str(&self.columns);
        s.font = ReaderFont::from_str(&self.font);
        s.spacing = ReaderSpacing::from_value(self.spacing);
        s.text_width = ReaderTextWidth::from_str(&self.text_width);
        s.justify_text = self.justify_text;
        s.indent_paragraphs = self.indent_paragraphs;
        s.bold_text = self.bold_text;
        s.character_spacing = self.character_spacing;
        s.word_spacing = self.word_spacing;
        s.margin = self.margin;
        s.selection_color_slot = self.selection_color_slot;
        s.highlight_color_slot = self.highlight_color_slot;
        s.gloss_color_slot = self.gloss_color;
        s.gloss_background_slot = self.gloss_background;
        s.word_color_slot = self.word_color;
        s.word_background_slot = self.word_background;
        s.gloss_enabled = self.gloss_enabled;
        s.font_size = Some(self.font_size / 2.0);
        s.gloss_font_size = self.gloss_size / 1.4;
        s.english_level = EnglishLevel::from_str(&self.english_level);
        Ok(())
    }

    pub fn font_family(&self) -> &'static str {
        match self.font.as_str() {
            "canela" => "Canela,Georgia,serif",
            "athelas" => "Athelas,Georgia,serif",
            "avenir_next" => "'Avenir Next',Avenir,sans-serif",
            "charter" => "Charter,'Bitstream Charter',serif",
            "georgia" => "Georgia,serif",
            "iowan" => "'Iowan Old Style',Palatino,serif",
            "palatino" => "Palatino,'Palatino Linotype',serif",
            "proxima_nova" => "'Proxima Nova',Arial,sans-serif",
            "publico" => "Publico,Georgia,serif",
            "san_francisco" => "-apple-system,BlinkMacSystemFont,Arial,sans-serif",
            "new_york" => "'New York',ui-serif,Georgia,serif",
            "seravek" => "Seravek,Arial,sans-serif",
            "times_new_roman" => "'Times New Roman',Times,serif",
            _ => "Georgia,'Songti SC','Noto Serif CJK SC',serif",
        }
    }

    pub fn scene(&self) -> (&'static str, &'static str) {
        if self.background == "theme" && self.theme == "dark" {
            return ("#4D4D4D", "#D9D9D9");
        }
        SCENES
            .iter()
            .find(|s| s.0 == self.background)
            .map(|s| (s.2, s.3))
            .unwrap_or(("#F7F4EA", "#2B2925"))
    }

    pub fn ui_style(&self, reader: bool) -> String {
        let (paper, ink) = if reader {
            self.scene()
        } else if self.theme == "dark" {
            ("#252729", "#E8E4DC")
        } else {
            ("#FAF7F0", "#302F2A")
        };
        let dark = if reader {
            self.background == "night" || self.background == "theme" && self.theme == "dark"
        } else {
            self.theme == "dark"
        };
        let (panel, muted, line, accent) = if dark {
            ("#303439", "#B4AD9E", "#4B4A46", "#D6B17D")
        } else {
            ("#FFFDF8", "#7A776D", "#E7E1D5", "#9C5039")
        };
        format!(
            "--paper:{paper};--ink:{ink};--panel:{panel};--muted:{muted};--line:{line};--accent:{accent};color-scheme:{}",
            if dark { "dark" } else { "light" }
        )
    }

    pub fn style(&self) -> String {
        let pen = [
            "#7FB2A688",
            "#4A90D988",
            "#E8875A88",
            "#D9C96A88",
            "#C1401B88",
            "#2C58C988",
            "#3B7A3D88",
            "#4B4A4788",
        ][self.selection_color_slot.min(7)];
        let colours = self.annotation_colours();
        format!(
            "--gloss-color:{};--gloss-background:{};--word-color:{};--word-background:{};--font-size:{}px;--gloss-size:{}px;--reader-font:{};--reader-leading:{};--reader-align:{};--reader-indent:{};--paragraph-gap:{};--reader-weight:{};--letter-spacing:{}em;--word-spacing:{}em;--selection-ink:{pen}",
            colours[0],
            colours[1],
            colours[2],
            colours[3],
            self.font_size,
            self.gloss_size,
            self.font_family(),
            self.spacing,
            if self.justify_text { "justify" } else { "left" },
            if self.indent_paragraphs { "2em" } else { "0" },
            if self.indent_paragraphs { "0" } else { "0.9em" },
            if self.bold_text { "600" } else { "400" },
            self.character_spacing,
            self.word_spacing
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn annotation_palette_and_slots_match_the_native_contract() {
        assert_eq!(
            BACKGROUND,
            &[
                ("无", "transparent"),
                ("奶黄", "#FFE9A8"),
                ("薄荷", "#CFE8D5"),
                ("雾蓝", "#D3E6F7")
            ]
        );
        let mut settings = ReaderSettings::default();
        let mut prefs = Preferences::from(&settings);
        prefs.gloss_background = 2;
        prefs.word_background = 3;
        prefs.apply(&mut settings).unwrap();
        assert_eq!(settings.gloss_background_slot, 2);
        assert_eq!(Preferences::from(&settings).word_background, 3);
    }
    #[test]
    fn every_visible_setting_round_trips_through_native_storage_types() {
        let mut p = Preferences {
            language: "en".into(),
            theme: "light".into(),
            background: "sage".into(),
            turn_mode: "slide".into(),
            columns: "two".into(),
            font: "seravek".into(),
            spacing: 2.15,
            text_width: "wide".into(),
            justify_text: true,
            indent_paragraphs: true,
            selection_color_slot: 7,
            highlight_color_slot: 6,
            font_size: 26.0,
            gloss_size: 13.3,
            ..Preferences::default()
        };
        let mut s = ReaderSettings::default();
        p.apply(&mut s).unwrap();
        assert_eq!(Preferences::from(&s), p);
        for font in ReaderFont::ALL {
            p.font = font.as_str().into();
            assert!(p.validate().is_ok());
        }
        for scene in SCENES {
            p.background = scene.0.into();
            assert!(p.validate().is_ok());
        }
    }
    #[test]
    fn client_settings_are_validated_before_saving() {
        let mut prefs = Preferences::default();
        assert!(prefs.validate().is_ok());
        prefs.font_size = f32::NAN;
        assert!(prefs.validate().is_err());
        prefs.font_size = 22.0;
        prefs.gloss_color = usize::MAX;
        assert!(prefs.validate().is_err());
        prefs.gloss_color = 0;
        prefs.english_level = "C9".into();
        assert!(prefs.validate().is_err());
        prefs.english_level = "A1".into();
        prefs.font = "url(javascript:alert(1))".into();
        assert!(prefs.validate().is_err());
    }
}
