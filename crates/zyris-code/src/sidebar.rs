//! What goes in the right sidebar — usage and tasks.
//!
//! Usage comes from `session_usage`. **Tasks are not that simple.** The `todo_change`
//! event carries only a `todo_item_id` and a status, **no body**, and `attacca_api` has no tool
//! to read the todo list. So it is scraped from the `todo_*` tool calls the agent makes:
//!
//! - The **result** of `todo_list` is the most accurate — the full list at that moment.
//! - The **arguments** of `todo_add` carry the new item's body.
//! - The arguments of `todo_update_status` only change the status.
//!
//! So what is shown here is "what the agent said through tools", not the server's ground truth.

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Pending,
    Running,
    Done,
}

impl TaskState {
    pub fn mark(self) -> &'static str {
        match self {
            TaskState::Pending => "○",
            TaskState::Running => "◐",
            TaskState::Done => "●",
        }
    }

    fn from_str(s: &str) -> TaskState {
        match s {
            "in_progress" | "running" => TaskState::Running,
            "done" | "completed" => TaskState::Done,
            _ => TaskState::Pending,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub id: Option<String>,
    pub text: String,
    pub state: TaskState,
}

/// The session's usage. A missing value means no turn has run yet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Usage {
    pub model: Option<String>,
    pub context_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub credits_used: Option<String>,
}

/// What the sidebar holds.
#[derive(Debug, Clone, Default)]
pub struct Sidebar {
    pub usage: Usage,
    pub tasks: Vec<Task>,
}

impl Sidebar {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cleared when switching sessions — stale numbers from the previous session would lie.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Reflects one `todo_*` tool call.
    pub fn apply_tool(&mut self, name: &str, arguments: &Value, result: Option<&Value>) {
        match name {
            // When a full list arrives, it is the ground truth.
            "todo_list" => {
                if let Some(items) = result.and_then(as_items) {
                    self.tasks = items;
                }
            }
            "todo_add" => {
                // If the result carries a full list, that is better.
                if let Some(items) = result.and_then(as_items) {
                    self.tasks = items;
                    return;
                }
                if let Some(text) = arguments.get("content").and_then(Value::as_str) {
                    self.tasks.push(Task {
                        id: result
                            .and_then(|r| r.get("id"))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        text: text.to_string(),
                        state: TaskState::Pending,
                    });
                }
            }
            "todo_update_status" => {
                if let Some(items) = result.and_then(as_items) {
                    self.tasks = items;
                    return;
                }
                let id = arguments.get("id").and_then(Value::as_str);
                let status = arguments.get("status").and_then(Value::as_str);
                if let (Some(id), Some(status)) = (id, status) {
                    if let Some(t) = self.tasks.iter_mut().find(|t| t.id.as_deref() == Some(id)) {
                        t.state = TaskState::from_str(status);
                    }
                }
            }
            "todo_remove" => {
                if let Some(items) = result.and_then(as_items) {
                    self.tasks = items;
                    return;
                }
                if let Some(id) = arguments.get("id").and_then(Value::as_str) {
                    self.tasks.retain(|t| t.id.as_deref() != Some(id));
                }
            }
            _ => {}
        }
    }

    /// A few items, unfinished ones first.
    pub fn visible_tasks(&self, limit: usize) -> Vec<&Task> {
        let mut v: Vec<&Task> = self.tasks.iter().filter(|t| t.state != TaskState::Done).collect();
        if v.len() < limit {
            v.extend(self.tasks.iter().filter(|t| t.state == TaskState::Done));
        }
        v.into_iter().take(limit).collect()
    }
}

/// Reads the item list from a tool result. Accepts an array, `{items: [...]}`, or both.
fn as_items(v: &Value) -> Option<Vec<Task>> {
    let arr = v.as_array().or_else(|| v.get("items")?.as_array())?;
    let tasks: Vec<Task> = arr
        .iter()
        .filter_map(|it| {
            let text = it.get("content").and_then(Value::as_str)?;
            Some(Task {
                id: it.get("id").and_then(Value::as_str).map(str::to_string),
                text: text.to_string(),
                state: it
                    .get("status")
                    .and_then(Value::as_str)
                    .map(TaskState::from_str)
                    .unwrap_or(TaskState::Pending),
            })
        })
        .collect();
    (!tasks.is_empty()).then_some(tasks)
}

/// How much context this model fits at once. **`None` when unknown, and then no maximum is shown.**
///
/// The server does not provide this — `ZUsage` only has what was used so far. So we guess from
/// the model name, and **guesses quietly go stale.** A new model just is not here, so the
/// maximum disappears; it never shows a wrong number. `ZYRIS_CODE_CONTEXT_MAX` overrides known-wrong values.
pub fn context_limit(model: Option<&str>) -> Option<i64> {
    if let Some(n) = std::env::var("ZYRIS_CODE_CONTEXT_MAX").ok().and_then(|v| v.parse().ok()) {
        return Some(n);
    }
    let m = model?.to_ascii_lowercase();
    // If the name bakes in the window size, that is the most accurate.
    if m.contains("1m") {
        return Some(1_000_000);
    }
    match () {
        _ if m.contains("claude") || m.contains("opus") || m.contains("sonnet") => Some(200_000),
        _ if m.contains("haiku") => Some(200_000),
        _ if m.contains("gemini") => Some(1_000_000),
        _ if m.contains("gpt-4o") => Some(128_000),
        _ => None,
    }
}

/// Big numbers, short. The sidebar is narrow.
///
/// When it lands evenly, drop the decimal and write `200k` — `200.0k` looks precise but reads badly.
pub fn compact(n: i64) -> String {
    let short = match n {
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1_000_000.0),
        n if n >= 1_000 => format!("{:.1}k", n as f64 / 1_000.0),
        n => return n.to_string(),
    };
    short.replace(".0", "")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_todo_list_result_replaces_everything() {
        let mut s = Sidebar::new();
        s.tasks.push(Task { id: None, text: "옛것".into(), state: TaskState::Pending });
        s.apply_tool(
            "todo_list",
            &json!({}),
            Some(&json!([
                {"id": "a", "content": "첫째", "status": "done"},
                {"id": "b", "content": "둘째", "status": "in_progress"}
            ])),
        );
        assert_eq!(s.tasks.len(), 2);
        assert_eq!(s.tasks[0].state, TaskState::Done);
        assert_eq!(s.tasks[1].state, TaskState::Running);
    }

    /// When the result has no list, fill in from the argument's body at least.
    #[test]
    fn todo_add_falls_back_to_its_argument() {
        let mut s = Sidebar::new();
        s.apply_tool("todo_add", &json!({"content": "새 할 일"}), None);
        assert_eq!(s.tasks.len(), 1);
        assert_eq!(s.tasks[0].text, "새 할 일");
        assert_eq!(s.tasks[0].state, TaskState::Pending);
    }

    #[test]
    fn updating_status_moves_the_right_task() {
        let mut s = Sidebar::new();
        s.apply_tool("todo_add", &json!({"content": "일"}), Some(&json!({"id": "x"})));
        s.apply_tool("todo_update_status", &json!({"id": "x", "status": "done"}), None);
        assert_eq!(s.tasks[0].state, TaskState::Done);
    }

    /// Unfinished items must come before finished ones — what to do now goes on top.
    #[test]
    fn unfinished_tasks_come_first() {
        let mut s = Sidebar::new();
        s.apply_tool(
            "todo_list",
            &json!({}),
            Some(&json!([
                {"id": "a", "content": "끝난 것", "status": "done"},
                {"id": "b", "content": "남은 것", "status": "pending"}
            ])),
        );
        let v = s.visible_tasks(1);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].text, "남은 것");
    }

    /// Switching sessions must clear. Stale numbers from the previous session would lie.
    #[test]
    fn clearing_drops_everything() {
        let mut s = Sidebar::new();
        s.usage.total_tokens = Some(100);
        s.apply_tool("todo_add", &json!({"content": "일"}), None);
        s.clear();
        assert!(s.tasks.is_empty());
        assert_eq!(s.usage.total_tokens, None);
    }

    #[test]
    fn big_numbers_get_short() {
        assert_eq!(compact(999), "999");
        assert_eq!(compact(1_500), "1.5k");
        assert_eq!(compact(2_400_000), "2.4M");
        assert_eq!(compact(200_000), "200k", "딱 떨어지면 소수점을 뗀다");
    }

    /// Unknown tools are ignored.
    #[test]
    fn an_unrelated_tool_changes_nothing() {
        let mut s = Sidebar::new();
        s.apply_tool("web_search", &json!({"query": "x"}), None);
        assert!(s.tasks.is_empty());
    }
}
