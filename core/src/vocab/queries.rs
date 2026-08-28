use super::models::*;
use sqlx::{Row, SqlitePool};

/// 覆盖式保存一本书的生词扫描结果
pub async fn save_book_words(
    pool: &SqlitePool,
    book_id: &str,
    words: &[BookWord],
) -> Result<(), String> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| format!("开启事务失败: {}", e))?;

    sqlx::query("DELETE FROM book_words WHERE book_id = ?")
        .bind(book_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("清理旧生词失败: {}", e))?;

    for word in words {
        sqlx::query(
            "INSERT OR REPLACE INTO book_words (book_id, lemma, context_sentence) VALUES (?, ?, ?)",
        )
        .bind(book_id)
        .bind(&word.lemma)
        .bind(&word.context_sentence)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("保存生词失败: {}", e))?;
    }

    tx.commit()
        .await
        .map_err(|e| format!("提交事务失败: {}", e))
}

pub async fn get_book_words(pool: &SqlitePool, book_id: &str) -> Result<Vec<BookWord>, String> {
    let rows = sqlx::query("SELECT lemma, context_sentence FROM book_words WHERE book_id = ?")
        .bind(book_id)
        .fetch_all(pool)
        .await
        .map_err(|e| format!("获取生词失败: {}", e))?;

    Ok(rows
        .iter()
        .map(|row| BookWord {
            lemma: row.get("lemma"),
            context_sentence: row.get("context_sentence"),
        })
        .collect())
}

/// 保存一批语境释义（按书），同时回填全局兜底缓存中缺失的词
pub async fn save_book_glosses(
    pool: &SqlitePool,
    book_id: &str,
    glosses: &[GlossInput],
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp_millis();
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| format!("开启事务失败: {}", e))?;

    for item in glosses {
        sqlx::query("INSERT OR REPLACE INTO book_glosses (book_id, lemma, gloss, created_at) VALUES (?, ?, ?, ?)")
            .bind(book_id)
            .bind(&item.lemma)
            .bind(&item.gloss)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("保存释义失败: {}", e))?;

        sqlx::query("INSERT OR IGNORE INTO word_glosses (lemma, gloss, level, created_at) VALUES (?, ?, ?, ?)")
            .bind(&item.lemma)
            .bind(&item.gloss)
            .bind(item.level)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("回填全局释义失败: {}", e))?;
    }

    tx.commit()
        .await
        .map_err(|e| format!("提交事务失败: {}", e))
}

pub async fn get_book_glosses(pool: &SqlitePool, book_id: &str) -> Result<Vec<WordGloss>, String> {
    let rows = sqlx::query("SELECT lemma, gloss FROM book_glosses WHERE book_id = ?")
        .bind(book_id)
        .fetch_all(pool)
        .await
        .map_err(|e| format!("获取释义失败: {}", e))?;

    Ok(rows
        .iter()
        .map(|row| WordGloss {
            lemma: row.get("lemma"),
            gloss: row.get("gloss"),
            level: None,
        })
        .collect())
}

/// 从全局兜底缓存查询一批词的释义
pub async fn get_word_glosses(
    pool: &SqlitePool,
    lemmas: &[String],
) -> Result<Vec<WordGloss>, String> {
    let mut result = Vec::new();
    for chunk in lemmas.chunks(500) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let sql = format!(
            "SELECT lemma, gloss, level FROM word_glosses WHERE lemma IN ({})",
            placeholders
        );
        let mut query = sqlx::query(&sql);
        for lemma in chunk {
            query = query.bind(lemma);
        }
        let rows = query
            .fetch_all(pool)
            .await
            .map_err(|e| format!("查询全局释义失败: {}", e))?;
        for row in rows.iter() {
            result.push(WordGloss {
                lemma: row.get("lemma"),
                gloss: row.get("gloss"),
                level: row.get("level"),
            });
        }
    }
    Ok(result)
}

pub async fn add_known_word(pool: &SqlitePool, lemma: &str, source: &str) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query("INSERT OR REPLACE INTO known_words (lemma, source, created_at) VALUES (?, ?, ?)")
        .bind(lemma)
        .bind(source)
        .bind(now)
        .execute(pool)
        .await
        .map_err(|e| format!("保存已认识单词失败: {}", e))?;
    Ok(())
}

pub async fn remove_known_word(pool: &SqlitePool, lemma: &str) -> Result<(), String> {
    sqlx::query("DELETE FROM known_words WHERE lemma = ?")
        .bind(lemma)
        .execute(pool)
        .await
        .map_err(|e| format!("移除已认识单词失败: {}", e))?;
    Ok(())
}

pub async fn get_known_words(pool: &SqlitePool) -> Result<Vec<String>, String> {
    let rows = sqlx::query("SELECT lemma FROM known_words")
        .fetch_all(pool)
        .await
        .map_err(|e| format!("获取已认识单词失败: {}", e))?;
    Ok(rows.iter().map(|row| row.get("lemma")).collect())
}

/// 加入生词本；已存在时不覆盖，返回是否新增
pub async fn vocab_book_add(pool: &SqlitePool, entry: &VocabBookAddInput) -> Result<bool, String> {
    let now = chrono::Utc::now().timestamp_millis();
    let result = sqlx::query(
        "INSERT OR IGNORE INTO vocab_book (lemma, word, gloss, level, context_sentence, book_id, status, review_count, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, 'learning', 0, ?, ?)",
    )
    .bind(&entry.lemma)
    .bind(&entry.word)
    .bind(&entry.gloss)
    .bind(entry.level)
    .bind(&entry.context_sentence)
    .bind(&entry.book_id)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| format!("加入生词本失败: {}", e))?;
    Ok(result.rows_affected() > 0)
}

pub async fn vocab_book_list(pool: &SqlitePool) -> Result<Vec<VocabBookEntry>, String> {
    let rows = sqlx::query(
        "SELECT lemma, word, gloss, level, context_sentence, book_id, status, review_count, created_at, updated_at
         FROM vocab_book ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("获取生词本失败: {}", e))?;

    Ok(rows
        .iter()
        .map(|row| VocabBookEntry {
            lemma: row.get("lemma"),
            word: row.get("word"),
            gloss: row.get("gloss"),
            level: row.get("level"),
            context_sentence: row.get("context_sentence"),
            book_id: row.get("book_id"),
            status: row.get("status"),
            review_count: row.get("review_count"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        })
        .collect())
}

pub async fn vocab_book_remove(pool: &SqlitePool, lemma: &str) -> Result<(), String> {
    let result = sqlx::query("DELETE FROM vocab_book WHERE lemma = ?")
        .bind(lemma)
        .execute(pool)
        .await
        .map_err(|e| format!("移除生词失败: {}", e))?;
    if result.rows_affected() == 0 {
        return Err("生词不存在，可能已被移除".to_string());
    }
    Ok(())
}

fn validate_vocab_status(status: &str) -> Result<(), String> {
    if status == "learning" || status == "mastered" {
        Ok(())
    } else {
        Err(format!("无效状态: {}", status))
    }
}

pub async fn vocab_book_set_status(
    pool: &SqlitePool,
    lemma: &str,
    status: &str,
) -> Result<(), String> {
    validate_vocab_status(status)?;
    let now = chrono::Utc::now().timestamp_millis();
    let result = sqlx::query("UPDATE vocab_book SET status = ?, updated_at = ? WHERE lemma = ?")
        .bind(status)
        .bind(now)
        .bind(lemma)
        .execute(pool)
        .await
        .map_err(|e| format!("更新生词状态失败: {}", e))?;
    if result.rows_affected() == 0 {
        return Err("生词不存在，可能已被移除".to_string());
    }
    Ok(())
}

pub async fn vocab_book_review(pool: &SqlitePool, lemma: &str) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp_millis();
    let result = sqlx::query(
        "UPDATE vocab_book SET review_count = review_count + 1, updated_at = ? WHERE lemma = ?",
    )
    .bind(now)
    .bind(lemma)
    .execute(pool)
    .await
    .map_err(|e| format!("记录复习失败: {}", e))?;
    if result.rows_affected() == 0 {
        return Err("生词不存在，可能已被移除".to_string());
    }
    Ok(())
}

/// 原子记录一次复习结果，同时更新学习状态。
pub async fn vocab_book_review_with_status(
    pool: &SqlitePool,
    lemma: &str,
    status: &str,
) -> Result<(), String> {
    validate_vocab_status(status)?;
    let now = chrono::Utc::now().timestamp_millis();
    let result = sqlx::query(
        "UPDATE vocab_book \
         SET status = ?, review_count = review_count + 1, updated_at = ? \
         WHERE lemma = ?",
    )
    .bind(status)
    .bind(now)
    .bind(lemma)
    .execute(pool)
    .await
    .map_err(|e| format!("记录复习结果失败: {}", e))?;
    if result.rows_affected() == 0 {
        return Err("生词不存在，可能已被移除".to_string());
    }
    Ok(())
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
            .block_on(
                sqlx::query(
                    "CREATE TABLE vocab_book (\
                        lemma TEXT PRIMARY KEY NOT NULL, word TEXT NOT NULL, gloss TEXT, \
                        level INTEGER, context_sentence TEXT, book_id TEXT, \
                        status TEXT NOT NULL DEFAULT 'learning', review_count INTEGER NOT NULL DEFAULT 0, \
                        created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL\
                    )",
                )
                .execute(&pool),
            )
            .unwrap();
        (runtime, pool)
    }

    #[test]
    fn vocab_review_updates_count_and_status_atomically() {
        let (runtime, pool) = pool();
        runtime.block_on(async {
            let input = VocabBookAddInput {
                lemma: "learn".to_string(),
                word: "learning".to_string(),
                gloss: Some("学习".to_string()),
                level: Some(1),
                context_sentence: Some("Keep learning.".to_string()),
                book_id: Some("book-1".to_string()),
            };
            assert!(vocab_book_add(&pool, &input).await.unwrap());
            assert!(!vocab_book_add(&pool, &input).await.unwrap());

            vocab_book_review_with_status(&pool, "learn", "mastered")
                .await
                .unwrap();
            let entries = vocab_book_list(&pool).await.unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].status, "mastered");
            assert_eq!(entries[0].review_count, 1);

            vocab_book_review_with_status(&pool, "learn", "learning")
                .await
                .unwrap();
            let entries = vocab_book_list(&pool).await.unwrap();
            assert_eq!(entries[0].status, "learning");
            assert_eq!(entries[0].review_count, 2);
        });
    }

    #[test]
    fn vocab_mutations_validate_status_and_missing_rows() {
        let (runtime, pool) = pool();
        runtime.block_on(async {
            assert!(vocab_book_set_status(&pool, "missing", "invalid")
                .await
                .is_err());
            assert!(vocab_book_set_status(&pool, "missing", "learning")
                .await
                .is_err());
            assert!(vocab_book_review(&pool, "missing").await.is_err());
            assert!(vocab_book_remove(&pool, "missing").await.is_err());
        });
    }
}
