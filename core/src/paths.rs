use std::path::PathBuf;

/// 应用数据目录。
/// 桌面：系统数据目录下 ReadWise（macOS: ~/Library/Application Support/ReadWise）；
/// iOS/Android：dirs 会解析到沙箱内的对应目录（M7 真机验证后如有偏差再调）。
pub fn app_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")))
        .join("ReadWise")
}
