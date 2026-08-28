use super::models::{DailyReading, StatisticsSnapshot};
use chrono::{Duration, Local};
use sqlx::{Row, SqlitePool};

/// 开始一次阅读会话。未正常退出留下的旧会话会在数据库初始化时恢复，
/// 因而这里始终创建一条独立记录。
pub async fn start_reading_session(pool: &SqlitePool, book_id: &str) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query(
        "INSERT INTO reading_sessions \
         (id, book_id, started_at, ended_at, duration_seconds, created_at, updated_at) \
         VALUES (?, ?, ?, NULL, 0, ?, ?)",
    )
    .bind(&id)
    .bind(book_id)
    .bind(now)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|error| format!("开始阅读计时失败: {error}"))?;
    Ok(id)
}

/// 保存仍在进行中的阅读时长，避免应用异常退出时丢失整段会话。
pub async fn checkpoint_reading_session(pool: &SqlitePool, id: &str) -> Result<(), String> {
    update_reading_session(pool, id, false).await
}

/// 结束阅读会话。重复结束是安全的，不会继续增加时长。
pub async fn finish_reading_session(pool: &SqlitePool, id: &str) -> Result<(), String> {
    update_reading_session(pool, id, true).await
}

async fn update_reading_session(pool: &SqlitePool, id: &str, finish: bool) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp_millis();
    let query = if finish {
        "UPDATE reading_sessions \
         SET ended_at = ?, \
             duration_seconds = MAX(duration_seconds, MAX(0, (? - started_at + 999) / 1000)), \
             updated_at = ? \
         WHERE id = ? AND ended_at IS NULL"
    } else {
        "UPDATE reading_sessions \
         SET duration_seconds = MAX(duration_seconds, MAX(0, (? - started_at + 999) / 1000)), \
             updated_at = ? \
         WHERE id = ? AND ended_at IS NULL"
    };
    let mut statement = sqlx::query(query);
    if finish {
        statement = statement.bind(now).bind(now).bind(now).bind(id);
    } else {
        statement = statement.bind(now).bind(now).bind(id);
    }
    statement
        .execute(pool)
        .await
        .map_err(|error| format!("保存阅读计时失败: {error}"))?;
    Ok(())
}

/// 把异常退出遗留的开放会话截断到最后一次 checkpoint。
pub async fn recover_open_reading_sessions(pool: &SqlitePool) -> Result<(), String> {
    sqlx::query(
        "UPDATE reading_sessions \
         SET ended_at = updated_at, \
             duration_seconds = MAX(duration_seconds, MAX(0, (updated_at - started_at) / 1000)) \
         WHERE ended_at IS NULL",
    )
    .execute(pool)
    .await
    .map_err(|error| format!("恢复阅读计时失败: {error}"))?;
    Ok(())
}

pub async fn statistics_snapshot(pool: &SqlitePool) -> Result<StatisticsSnapshot, String> {
    let summary = sqlx::query(
        "SELECT \
            COALESCE((SELECT SUM(duration_seconds) FROM reading_sessions), 0) AS total_seconds, \
            COALESCE((SELECT SUM(duration_seconds) FROM reading_sessions \
                WHERE DATE(started_at / 1000, 'unixepoch', 'localtime') = DATE('now', 'localtime')), 0) AS today_seconds, \
            COALESCE((SELECT COUNT(DISTINCT DATE(started_at / 1000, 'unixepoch', 'localtime')) \
                FROM reading_sessions WHERE duration_seconds > 0), 0) AS active_days, \
            COALESCE((SELECT COUNT(*) FROM books), 0) AS total_books, \
            COALESCE((SELECT COUNT(*) FROM book_status WHERE status = 'completed'), 0) AS completed_books, \
            COALESCE((SELECT COUNT(*) FROM vocab_book), 0) AS vocab_total, \
            COALESCE((SELECT COUNT(*) FROM vocab_book WHERE status = 'learning'), 0) AS vocab_learning, \
            COALESCE((SELECT COUNT(*) FROM vocab_book WHERE status = 'mastered'), 0) AS vocab_mastered, \
            COALESCE((SELECT SUM(review_count) FROM vocab_book), 0) AS total_reviews",
    )
    .fetch_one(pool)
    .await
    .map_err(|error| format!("读取统计摘要失败: {error}"))?;

    let rows = sqlx::query(
        "SELECT DATE(started_at / 1000, 'unixepoch', 'localtime') AS day, \
                SUM(duration_seconds) AS duration_seconds \
         FROM reading_sessions \
         WHERE started_at >= CAST(strftime('%s', 'now', 'localtime', 'start of day', '-6 days', 'utc') AS INTEGER) * 1000 \
         GROUP BY day",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| format!("读取每日阅读统计失败: {error}"))?;

    let by_date = rows
        .iter()
        .map(|row| {
            (
                row.get::<String, _>("day"),
                row.get::<i64, _>("duration_seconds"),
            )
        })
        .collect::<std::collections::HashMap<_, _>>();
    let today = Local::now().date_naive();
    let daily_reading = (0..7)
        .rev()
        .map(|days_ago| {
            let date = today - Duration::days(days_ago);
            let date = date.format("%Y-%m-%d").to_string();
            DailyReading {
                duration_seconds: by_date.get(&date).copied().unwrap_or(0),
                date,
            }
        })
        .collect();

    Ok(StatisticsSnapshot {
        total_reading_seconds: summary.get("total_seconds"),
        today_reading_seconds: summary.get("today_seconds"),
        active_days: summary.get("active_days"),
        total_books: summary.get("total_books"),
        completed_books: summary.get("completed_books"),
        vocab_total: summary.get("vocab_total"),
        vocab_learning: summary.get("vocab_learning"),
        vocab_mastered: summary.get("vocab_mastered"),
        total_reviews: summary.get("total_reviews"),
        daily_reading,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool() -> (tokio::runtime::Runtime, SqlitePool) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let pool = runtime
            .block_on(SqlitePool::connect("sqlite::memory:"))
            .unwrap();
        runtime
            .block_on(sqlx::query(include_str!("../schema.sql")).execute(&pool))
            .unwrap();
        (runtime, pool)
    }

    #[test]
    fn session_lifecycle_is_idempotent() {
        let (runtime, pool) = pool();
        runtime.block_on(async {
            let now = chrono::Utc::now().timestamp_millis();
            sqlx::query(
                "INSERT INTO books (id, title, author, format, file_path, file_size, language, created_at, updated_at) \
                 VALUES ('book', 'Book', 'Author', 'epub', 'book.epub', 1, 'en', ?, ?)",
            )
            .bind(now)
            .bind(now)
            .execute(&pool)
            .await
            .unwrap();
            let id = start_reading_session(&pool, "book").await.unwrap();
            checkpoint_reading_session(&pool, &id).await.unwrap();
            finish_reading_session(&pool, &id).await.unwrap();
            finish_reading_session(&pool, &id).await.unwrap();
            let row = sqlx::query(
                "SELECT ended_at, duration_seconds FROM reading_sessions WHERE id = ?",
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert!(row.get::<Option<i64>, _>("ended_at").is_some());
            assert!(row.get::<i64, _>("duration_seconds") >= 0);
        });
    }

    #[test]
    fn snapshot_fills_all_seven_days_and_vocab_counts() {
        let (runtime, pool) = pool();
        runtime.block_on(async {
            let now = chrono::Utc::now().timestamp_millis();
            sqlx::query(
                "INSERT INTO vocab_book \
                 (lemma, word, status, review_count, created_at, updated_at) VALUES \
                 ('read', 'read', 'learning', 2, ?, ?), \
                 ('learn', 'learned', 'mastered', 3, ?, ?)",
            )
            .bind(now)
            .bind(now)
            .bind(now)
            .bind(now)
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO books (id, title, author, format, file_path, file_size, language, created_at, updated_at) \
                 VALUES ('book', 'Book', 'Author', 'epub', 'book.epub', 1, 'en', ?, ?)",
            )
            .bind(now)
            .bind(now)
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO reading_sessions \
                 (id, book_id, started_at, ended_at, duration_seconds, created_at, updated_at) \
                 VALUES ('session', 'book', ?, ?, 120, ?, ?)",
            )
            .bind(now)
            .bind(now + 120_000)
            .bind(now)
            .bind(now + 120_000)
            .execute(&pool)
            .await
            .unwrap();
            let snapshot = statistics_snapshot(&pool).await.unwrap();
            assert_eq!(snapshot.daily_reading.len(), 7);
            assert_eq!(snapshot.total_reading_seconds, 120);
            assert_eq!(snapshot.today_reading_seconds, 120);
            assert_eq!(snapshot.daily_reading.last().unwrap().duration_seconds, 120);
            assert_eq!(snapshot.vocab_total, 2);
            assert_eq!(snapshot.vocab_learning, 1);
            assert_eq!(snapshot.vocab_mastered, 1);
            assert_eq!(snapshot.total_reviews, 5);
        });
    }
}
