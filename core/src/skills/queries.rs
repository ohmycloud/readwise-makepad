use super::models::Skill;
use sqlx::{Row, SqlitePool};

fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Skill, String> {
    Ok(Skill {
        id: row
            .try_get("id")
            .map_err(|e| format!("读取技能 id 失败: {e}"))?,
        name: row
            .try_get("name")
            .map_err(|e| format!("读取技能名称失败: {e}"))?,
        content: row
            .try_get("content")
            .map_err(|e| format!("读取技能内容失败: {e}"))?,
        is_active: row
            .try_get::<i64, _>("is_active")
            .map_err(|e| format!("读取技能状态失败: {e}"))?
            != 0,
        is_system: row
            .try_get::<i64, _>("is_system")
            .map_err(|e| format!("读取技能类型失败: {e}"))?
            != 0,
        created_at: row
            .try_get("created_at")
            .map_err(|e| format!("读取技能创建时间失败: {e}"))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|e| format!("读取技能更新时间失败: {e}"))?,
    })
}

fn validate<'a>(name: &'a str, content: &'a str) -> Result<(&'a str, &'a str), String> {
    let name = name.trim();
    let content = content.trim();
    if name.is_empty() {
        return Err("技能名称不能为空".to_string());
    }
    if content.is_empty() {
        return Err("技能内容不能为空".to_string());
    }
    Ok((name, content))
}

fn write_error(operation: &str, error: sqlx::Error) -> String {
    let detail = error.to_string();
    if detail.contains("UNIQUE constraint failed: skills.name") {
        "技能名称已存在，请换一个名称".to_string()
    } else {
        format!("{operation}失败: {detail}")
    }
}

pub async fn list_skills(pool: &SqlitePool) -> Result<Vec<Skill>, String> {
    let rows = sqlx::query(
        "SELECT id, name, content, is_active, is_system, created_at, updated_at \
         FROM skills ORDER BY is_system DESC, updated_at DESC, name ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("读取技能列表失败: {e}"))?;
    rows.iter().map(from_row).collect()
}

pub async fn create_skill(
    pool: &SqlitePool,
    name: &str,
    content: &str,
    is_active: bool,
) -> Result<Skill, String> {
    let (name, content) = validate(name, content)?;
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query(
        "INSERT INTO skills (id, name, content, is_active, is_system, created_at, updated_at) \
         VALUES (?, ?, ?, ?, 0, ?, ?)",
    )
    .bind(&id)
    .bind(name)
    .bind(content)
    .bind(i64::from(is_active))
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| write_error("创建技能", e))?;
    get_skill(pool, &id).await
}

pub async fn update_skill(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    content: &str,
    is_active: bool,
) -> Result<Skill, String> {
    let (name, content) = validate(name, content)?;
    let existing = get_skill(pool, id).await?;
    if existing.is_system && existing.name != name {
        return Err("系统技能名称不可修改".to_string());
    }
    let now = chrono::Utc::now().timestamp_millis();
    let result = sqlx::query(
        "UPDATE skills SET name = ?, content = ?, is_active = ?, updated_at = ? WHERE id = ?",
    )
    .bind(name)
    .bind(content)
    .bind(i64::from(is_active))
    .bind(now)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| write_error("更新技能", e))?;
    if result.rows_affected() == 0 {
        return Err("技能不存在，可能已被删除".to_string());
    }
    get_skill(pool, id).await
}

pub async fn set_skill_active(pool: &SqlitePool, id: &str, is_active: bool) -> Result<(), String> {
    let result = sqlx::query("UPDATE skills SET is_active = ?, updated_at = ? WHERE id = ?")
        .bind(i64::from(is_active))
        .bind(chrono::Utc::now().timestamp_millis())
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| format!("更新技能状态失败: {e}"))?;
    if result.rows_affected() == 0 {
        return Err("技能不存在，可能已被删除".to_string());
    }
    Ok(())
}

pub async fn delete_skill(pool: &SqlitePool, id: &str) -> Result<(), String> {
    let result = sqlx::query("DELETE FROM skills WHERE id = ? AND is_system = 0")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| format!("删除技能失败: {e}"))?;
    if result.rows_affected() == 0 {
        return Err("系统技能不可删除，或该技能已不存在".to_string());
    }
    Ok(())
}

async fn get_skill(pool: &SqlitePool, id: &str) -> Result<Skill, String> {
    let row = sqlx::query(
        "SELECT id, name, content, is_active, is_system, created_at, updated_at \
         FROM skills WHERE id = ?",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("读取技能失败: {e}"))?;
    from_row(&row)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE skills (\
                id TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL UNIQUE, content TEXT NOT NULL, \
                is_active INTEGER DEFAULT 1, is_system INTEGER DEFAULT 0, \
                created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL\
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[test]
    fn custom_skill_crud_and_validation() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let pool = pool().await;
            assert_eq!(
                create_skill(&pool, " ", "content", true).await.unwrap_err(),
                "技能名称不能为空"
            );
            let skill = create_skill(&pool, " 章节总结 ", " 先提炼主题 ", true)
                .await
                .unwrap();
            assert_eq!(skill.name, "章节总结");
            assert_eq!(skill.content, "先提炼主题");
            assert!(skill.is_active);
            assert!(!skill.is_system);

            let updated = update_skill(&pool, &skill.id, "章节精读", "解释并举例", false)
                .await
                .unwrap();
            assert_eq!(updated.name, "章节精读");
            assert!(!updated.is_active);
            set_skill_active(&pool, &skill.id, true).await.unwrap();
            assert!(list_skills(&pool).await.unwrap()[0].is_active);
            delete_skill(&pool, &skill.id).await.unwrap();
            assert!(list_skills(&pool).await.unwrap().is_empty());
        });
    }

    #[test]
    fn system_skill_cannot_be_deleted_and_names_are_unique() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let pool = pool().await;
            let now = chrono::Utc::now().timestamp_millis();
            sqlx::query(
                "INSERT INTO skills (id, name, content, is_active, is_system, created_at, updated_at) \
                 VALUES ('system', '系统提示词', 'system', 1, 1, ?, ?)",
            )
            .bind(now)
            .bind(now)
            .execute(&pool)
            .await
            .unwrap();
            assert!(delete_skill(&pool, "system").await.is_err());
            assert_eq!(
                update_skill(&pool, "system", "改名", "updated", true)
                    .await
                    .unwrap_err(),
                "系统技能名称不可修改"
            );
            let updated = update_skill(&pool, "system", "系统提示词", "updated", true)
                .await
                .unwrap();
            assert_eq!(updated.content, "updated");
            let error = create_skill(&pool, "系统提示词", "duplicate", true)
                .await
                .unwrap_err();
            assert_eq!(error, "技能名称已存在，请换一个名称");
        });
    }
}
