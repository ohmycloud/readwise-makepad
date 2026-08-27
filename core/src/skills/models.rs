#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub content: String,
    pub is_active: bool,
    pub is_system: bool,
    pub created_at: i64,
    pub updated_at: i64,
}
