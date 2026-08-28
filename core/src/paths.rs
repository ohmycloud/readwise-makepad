use std::path::{Path, PathBuf};

/// 应用数据目录。
/// 桌面：系统数据目录下 ReadWise（macOS: ~/Library/Application Support/ReadWise）；
/// 没有平台显式路径时回退到 dirs/home/current directory。
pub fn app_data_dir() -> PathBuf {
    app_data_dir_from_platform(None)
}

/// 优先使用 GUI 平台提供的可写沙箱根目录。
/// Makepad 在 iOS 返回 Application Support，在 Android 返回 Activity.getFilesDir()；
/// 这比依赖移动端进程是否设置 HOME/XDG 环境变量稳定。
pub fn app_data_dir_from_platform(platform_data_dir: Option<&Path>) -> PathBuf {
    platform_data_dir
        .map(Path::to_path_buf)
        .or_else(dirs::data_dir)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ReadWise")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_sandbox_path_has_a_stable_app_subdirectory() {
        assert_eq!(
            app_data_dir_from_platform(Some(Path::new("sandbox-root"))),
            PathBuf::from("sandbox-root").join("ReadWise")
        );
    }
}
