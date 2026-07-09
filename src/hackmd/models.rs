use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{error::HackMdError, paths::patch_path};

pub(crate) const DEFAULT_LIST_LIMIT: usize = 50;
pub(crate) const MAX_LIST_LIMIT: usize = 200;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Workspace {
    #[default]
    Personal,
    Team {
        team_path: String,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct WorkspaceInput {
    #[serde(default)]
    pub(crate) workspace: Workspace,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ListNotesInput {
    #[serde(default)]
    pub(crate) workspace: Option<Workspace>,
    pub(crate) query: Option<String>,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
    pub(crate) folder_id: Option<String>,
    pub(crate) limit: Option<usize>,
    pub(crate) offset: Option<usize>,
    pub(crate) sort: Option<NoteSort>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NoteSort {
    #[default]
    LastChangedDesc,
    LastChangedAsc,
    CreatedDesc,
    CreatedAsc,
    TitleAsc,
}

#[derive(Debug, Deserialize)]
pub(crate) struct NoteRefInput {
    #[serde(default)]
    pub(crate) workspace: Workspace,
    pub(crate) note_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateNoteInput {
    #[serde(default)]
    pub(crate) workspace: Workspace,
    #[serde(default)]
    pub(crate) note: Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EditNoteInput {
    #[serde(default)]
    pub(crate) workspace: Workspace,
    pub(crate) note_id: String,
    pub(crate) patch: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateNoteInput {
    #[serde(default)]
    pub(crate) workspace: Workspace,
    pub(crate) note_id: String,
    pub(crate) fields: Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateFolderInput {
    #[serde(default)]
    pub(crate) workspace: Workspace,
    #[serde(default)]
    pub(crate) folder: Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateFolderInput {
    #[serde(default)]
    pub(crate) workspace: Workspace,
    pub(crate) folder_id: String,
    pub(crate) fields: Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FolderRefInput {
    #[serde(default)]
    pub(crate) workspace: Workspace,
    pub(crate) folder_id: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ListNotesOutput {
    pub(crate) total: usize,
    pub(crate) offset: usize,
    pub(crate) limit: usize,
    pub(crate) notes: Vec<NoteSummary>,
}

#[derive(Debug, Serialize)]
pub(crate) struct NoteSummary {
    id: String,
    short_id: Option<String>,
    title: String,
    description: String,
    pub(crate) tags: Vec<String>,
    workspace: Workspace,
    pub(crate) patch_path: String,
    pub(crate) folder_ids: Vec<String>,
    pub(crate) created_at: SortableNumber,
    pub(crate) last_changed_at: SortableNumber,
    publish_link: Option<String>,
    permalink: Option<String>,
}

impl NoteSummary {
    pub(crate) fn title_sort_key(&self) -> String {
        self.title.to_lowercase()
    }

    pub(crate) fn matches_query(&self, query: &str) -> bool {
        normalize_search(&self.id).contains(query)
            || self
                .short_id
                .as_deref()
                .is_some_and(|short_id| normalize_search(short_id).contains(query))
            || normalize_search(&self.title).contains(query)
            || normalize_search(&self.description).contains(query)
            || self
                .tags
                .iter()
                .any(|tag| normalize_search(tag).contains(query))
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub(crate) struct SortableNumber(i64);

#[derive(Debug, Serialize)]
pub(crate) struct NoteOutput {
    note_id: String,
    patch_path: String,
    content: Option<String>,
    note: Value,
}

#[derive(Debug, Serialize)]
pub(crate) struct EditNoteOutput {
    pub(crate) note_id: String,
    pub(crate) patch_path: String,
    pub(crate) changed: bool,
    pub(crate) content: String,
}

pub(crate) fn note_summary(workspace: &Workspace, note: Value) -> NoteSummary {
    let id = string_field(&note, "id").unwrap_or_default();
    NoteSummary {
        short_id: string_field(&note, "shortId"),
        title: string_field(&note, "title").unwrap_or_default(),
        description: string_field(&note, "description").unwrap_or_default(),
        tags: string_array_field(&note, "tags"),
        patch_path: patch_path(workspace, &id),
        folder_ids: folder_ids(&note),
        created_at: SortableNumber(number_field(&note, "createdAt").unwrap_or_default()),
        last_changed_at: SortableNumber(number_field(&note, "lastChangedAt").unwrap_or_default()),
        publish_link: string_field(&note, "publishLink"),
        permalink: string_field(&note, "permalink"),
        workspace: workspace.clone(),
        id,
    }
}

pub(crate) fn note_output(workspace: &Workspace, note_id: &str, note: Value) -> NoteOutput {
    NoteOutput {
        note_id: note_id.to_owned(),
        patch_path: patch_path(workspace, note_id),
        content: note_content(&note).ok(),
        note,
    }
}

pub(crate) fn note_content(note: &Value) -> Result<String, HackMdError> {
    ["content", "text", "markdown"]
        .iter()
        .find_map(|field| note.get(*field)?.as_str().map(ToOwned::to_owned))
        .ok_or_else(|| {
            HackMdError::Api("HackMD note response did not include editable content".to_owned())
        })
}

pub(crate) fn normalize_search(value: &str) -> String {
    value.trim().to_lowercase()
}

fn string_field(value: &Value, field: &str) -> Option<String> {
    value.get(field)?.as_str().map(ToOwned::to_owned)
}

fn string_array_field(value: &Value, field: &str) -> Vec<String> {
    value
        .get(field)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str().map(ToOwned::to_owned))
        .collect()
}

fn number_field(value: &Value, field: &str) -> Option<i64> {
    value.get(field).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
            .or_else(|| value.as_f64().map(|value| value as i64))
    })
}

fn folder_ids(note: &Value) -> Vec<String> {
    note.get("folderPaths")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|folder| folder.get("id").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ListNotesInput, Workspace, note_summary};

    #[test]
    fn note_summary_includes_patch_path_and_folder_ids() {
        let summary = note_summary(
            &Workspace::Personal,
            json!({
                "id": "abc",
                "shortId": "short",
                "title": "Roadmap",
                "description": "Q3",
                "tags": ["planning"],
                "createdAt": 1,
                "lastChangedAt": 2,
                "folderPaths": [{ "id": "folder-1" }]
            }),
        );

        assert_eq!(summary.patch_path, "notes/abc.md");
        assert_eq!(summary.folder_ids, vec!["folder-1"]);
        assert!(summary.matches_query("road"));
        assert!(summary.matches_query("planning"));
    }

    #[test]
    fn list_input_defaults_workspace() -> anyhow::Result<()> {
        let input: ListNotesInput = serde_json::from_value(json!({}))?;
        assert!(input.workspace.is_none());
        Ok(())
    }
}
