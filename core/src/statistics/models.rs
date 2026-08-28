#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyReading {
    /// 本地日期，格式为 YYYY-MM-DD。
    pub date: String,
    pub duration_seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatisticsSnapshot {
    pub total_reading_seconds: i64,
    pub today_reading_seconds: i64,
    pub active_days: i64,
    pub total_books: i64,
    pub completed_books: i64,
    pub vocab_total: i64,
    pub vocab_learning: i64,
    pub vocab_mastered: i64,
    pub total_reviews: i64,
    pub daily_reading: Vec<DailyReading>,
}
