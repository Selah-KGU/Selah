use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WidgetClass {
    pub day: i32,
    pub period: i32,
    pub name: String,
    pub room: String,
    pub start_minutes: i32,
    pub end_minutes: i32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WidgetTodo {
    pub title: String,
    pub course: String,
    pub due_unix: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WidgetSnapshot {
    pub summary: String,
    pub classes: Vec<WidgetClass>,
    pub todos: Vec<WidgetTodo>,
}

#[derive(Debug, Clone)]
pub(crate) struct ClassSource {
    pub day: i32,
    pub period: i32,
    pub name: String,
    pub room: String,
    pub cancelled: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct TodoSource {
    pub title: String,
    pub course: String,
    pub status: String,
    pub deadline: String,
}
