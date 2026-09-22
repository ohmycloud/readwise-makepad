#[cfg(target_os = "android")]
mod android_import;
mod app;
mod chapter_image;
#[cfg(target_os = "macos")]
mod desktop_import;
#[cfg(target_os = "macos")]
mod desktop_window;
mod i18n;
mod mark_span;
#[cfg(target_os = "ios")]
mod mobile_import;
mod reader_columns;
mod reader_hit;
mod reader_image;
mod reader_inline_code;
mod reader_link;
mod reader_lists;
mod reader_page_turn;
mod reader_scroll;
mod reader_text;
mod reader_touch;
mod ruby_word;
mod sample_chapter;

/// 零依赖的最小 logger：把 core（log 门面）的日志打到 stderr。
#[cfg(not(any(target_os = "android", target_env = "ohos")))]
struct StderrLogger;

#[cfg(not(any(target_os = "android", target_env = "ohos")))]
impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info
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
