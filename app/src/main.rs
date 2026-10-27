#[cfg(target_os = "android")]
mod android_import;
#[cfg(target_os = "android")]
mod android_lan_transfer;
mod app;
mod app_ui_index;
mod chapter_image;
#[cfg(target_os = "macos")]
mod desktop_import;
#[cfg(target_os = "macos")]
mod desktop_window;
mod i18n;
mod lan_transfer;
mod mark_span;
#[cfg(target_os = "ios")]
mod mobile_import;
mod reader_book_pages;
mod reader_columns;
mod reader_fonts;
mod reader_hit;
mod reader_image;
mod reader_image_cache;
mod reader_inline_code;
mod reader_link;
mod reader_lists;
mod reader_math;
mod reader_page_turn;
mod reader_paint;
mod reader_paragraph;
mod reader_scroll;
mod reader_selection;
mod reader_settings_panel;
mod reader_text;
mod reader_touch;
mod reader_word;
mod reader_word_card;
mod ruby_word;
mod sample_chapter;
mod settings_swatch;
mod shelf_list;

/// 零依赖的最小 logger：把 core（log 门面）的日志打到 stderr。
#[cfg(not(any(target_os = "android", target_env = "ohos")))]
struct StderrLogger;

#[cfg(not(any(target_os = "android", target_env = "ohos")))]
impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        // Embedded-font/CMap diagnostics are verbose and synchronous on stderr.
        // Keep parser warnings/errors, without dumping font tables in normal use.
        let limit =
            if metadata.target() == "pdf_oxide" || metadata.target().starts_with("pdf_oxide::") {
                log::Level::Warn
            } else {
                log::Level::Info
            };
        metadata.level() <= limit
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!(
                "[{}] {}: {}",
                record.level(),
                record.target(),
                record.args()
            );
        }
    }

    fn flush(&self) {}
}

#[cfg(not(any(target_os = "android", target_env = "ohos")))]
static LOGGER: StderrLogger = StderrLogger;

#[cfg(not(any(target_os = "android", target_env = "ohos")))]
fn main() {
    let _ = log::set_logger(&LOGGER).map(|_| log::set_max_level(log::LevelFilter::Info));
    app::app_main()
}

#[cfg(all(test, not(any(target_os = "android", target_env = "ohos"))))]
mod logger_tests {
    use super::*;
    use log::Log;

    #[test]
    fn pdf_font_diagnostics_are_quiet_but_warnings_and_app_info_remain() {
        let enabled = |target, level| {
            LOGGER.enabled(&log::Metadata::builder().target(target).level(level).build())
        };
        assert!(!enabled("pdf_oxide::fonts::font_dict", log::Level::Info));
        assert!(enabled("pdf_oxide::fonts::font_dict", log::Level::Warn));
        assert!(enabled("pdf_oxide", log::Level::Error));
        assert!(enabled("readwise_core::books", log::Level::Info));
    }
}
