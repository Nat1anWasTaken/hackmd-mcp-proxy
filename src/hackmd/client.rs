use std::cmp::Reverse;

use reqwest::Url;
use serde_json::{Value, json};

use super::{
    error::HackMdError,
    models::{
        CreateFolderInput, CreateNoteInput, DEFAULT_LIST_LIMIT, EditNoteInput, EditNoteOutput,
        FolderRefInput, ListNotesInput, ListNotesOutput, NoteRefInput, NoteSort, UpdateFolderInput,
        UpdateNoteInput, Workspace, normalize_search, note_content, note_output, note_summary,
    },
    paths::{folder_item_path, folders_collection_path, note_item_path, notes_collection_path},
};
use crate::patch;

#[derive(Debug)]
pub(crate) struct HackMdClient<'a> {
    client: &'a reqwest::Client,
    base_url: Url,
    token: &'a str,
}

impl<'a> HackMdClient<'a> {
    pub(crate) fn new(
        client: &'a reqwest::Client,
        hackmd_api_url: &str,
        token: &'a str,
    ) -> Result<Self, HackMdError> {
        Ok(Self {
            client,
            base_url: Url::parse(&format!("{}/", hackmd_api_url.trim_end_matches('/')))?,
            token,
        })
    }

    async fn get_me(&self) -> Result<Value, HackMdError> {
        self.get("me").await
    }

    pub(crate) async fn list_notes(
        &self,
        input: ListNotesInput,
    ) -> Result<ListNotesOutput, HackMdError> {
        let workspace = input.workspace.unwrap_or_default();
        let mut notes = self
            .get_json(&notes_collection_path(&workspace))
            .await?
            .into_iter()
            .map(|value| note_summary(&workspace, value))
            .collect::<Vec<_>>();

        if let Some(query) = input
            .query
            .as_deref()
            .map(normalize_search)
            .filter(|query| !query.is_empty())
        {
            notes.retain(|note| note.matches_query(&query));
        }
        if !input.tags.is_empty() {
            let tags = input
                .tags
                .iter()
                .map(|tag| tag.to_lowercase())
                .collect::<Vec<_>>();
            notes.retain(|note| {
                tags.iter().all(|tag| {
                    note.tags
                        .iter()
                        .any(|candidate| candidate.to_lowercase() == *tag)
                })
            });
        }
        if let Some(folder_id) = input.folder_id.as_deref() {
            notes.retain(|note| {
                note.folder_ids
                    .iter()
                    .any(|candidate| candidate == folder_id)
            });
        }

        match input.sort.unwrap_or_default() {
            NoteSort::LastChangedDesc => notes.sort_by_key(|note| Reverse(note.last_changed_at)),
            NoteSort::LastChangedAsc => notes.sort_by_key(|note| note.last_changed_at),
            NoteSort::CreatedDesc => notes.sort_by_key(|note| Reverse(note.created_at)),
            NoteSort::CreatedAsc => notes.sort_by_key(|note| note.created_at),
            NoteSort::TitleAsc => notes.sort_by_key(|note| note.title_sort_key()),
        }

        let total = notes.len();
        let offset = input.offset.unwrap_or(0).min(total);
        let limit = input
            .limit
            .unwrap_or(DEFAULT_LIST_LIMIT)
            .min(super::models::MAX_LIST_LIMIT);
        let notes = notes.into_iter().skip(offset).take(limit).collect();

        Ok(ListNotesOutput {
            total,
            offset,
            limit,
            notes,
        })
    }

    pub(crate) async fn get_note(&self, input: NoteRefInput) -> Result<Value, HackMdError> {
        let note = self
            .get(&note_item_path(&input.workspace, &input.note_id))
            .await?;
        Ok(json!(note_output(&input.workspace, &input.note_id, note)))
    }

    pub(crate) async fn create_note(&self, input: CreateNoteInput) -> Result<Value, HackMdError> {
        let path = notes_collection_path(&input.workspace);
        self.post(&path, json_body(input.note)).await
    }

    pub(crate) async fn edit_note(
        &self,
        input: EditNoteInput,
    ) -> Result<EditNoteOutput, HackMdError> {
        let note = self
            .get(&note_item_path(&input.workspace, &input.note_id))
            .await?;
        let content = note_content(&note)?;
        let patch_path = super::paths::patch_path(&input.workspace, &input.note_id);
        let updated_content = patch::apply_note_patch(&content, &input.patch, &patch_path)?;
        if updated_content != content {
            self.patch(
                &note_item_path(&input.workspace, &input.note_id),
                json!({ "content": updated_content }),
            )
            .await?;
        }
        Ok(EditNoteOutput {
            note_id: input.note_id,
            patch_path,
            changed: updated_content != content,
            content: updated_content,
        })
    }

    pub(crate) async fn update_note(&self, input: UpdateNoteInput) -> Result<Value, HackMdError> {
        if input
            .fields
            .as_object()
            .is_none_or(|fields| fields.is_empty())
        {
            return Err(HackMdError::InvalidRequest(
                "fields must contain at least one note property".to_owned(),
            ));
        }
        self.patch(
            &note_item_path(&input.workspace, &input.note_id),
            input.fields,
        )
        .await
    }

    pub(crate) async fn delete_note(&self, input: NoteRefInput) -> Result<Value, HackMdError> {
        self.delete(&note_item_path(&input.workspace, &input.note_id))
            .await
    }

    pub(crate) async fn list_folders(&self, workspace: Workspace) -> Result<Value, HackMdError> {
        self.get(&folders_collection_path(&workspace)).await
    }

    pub(crate) async fn create_folder(
        &self,
        input: CreateFolderInput,
    ) -> Result<Value, HackMdError> {
        self.post(&folders_collection_path(&input.workspace), input.folder)
            .await
    }

    pub(crate) async fn update_folder(
        &self,
        input: UpdateFolderInput,
    ) -> Result<Value, HackMdError> {
        if input
            .fields
            .as_object()
            .is_none_or(|fields| fields.is_empty())
        {
            return Err(HackMdError::InvalidRequest(
                "fields must contain at least one folder property".to_owned(),
            ));
        }
        self.patch(
            &folder_item_path(&input.workspace, &input.folder_id),
            input.fields,
        )
        .await
    }

    pub(crate) async fn delete_folder(&self, input: FolderRefInput) -> Result<Value, HackMdError> {
        self.delete(&folder_item_path(&input.workspace, &input.folder_id))
            .await
    }

    async fn get(&self, path: &str) -> Result<Value, HackMdError> {
        self.send_json(self.client.get(self.url(path)?)).await
    }

    async fn get_json(&self, path: &str) -> Result<Vec<Value>, HackMdError> {
        let value = self.get(path).await?;
        value.as_array().cloned().ok_or_else(|| {
            HackMdError::Api(format!("expected array response for HackMD path {path}"))
        })
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value, HackMdError> {
        self.send_json(self.client.post(self.url(path)?).json(&body))
            .await
    }

    async fn patch(&self, path: &str, body: Value) -> Result<Value, HackMdError> {
        self.send_json(self.client.patch(self.url(path)?).json(&body))
            .await
    }

    async fn delete(&self, path: &str) -> Result<Value, HackMdError> {
        self.send_json(self.client.delete(self.url(path)?)).await
    }

    async fn send_json(&self, request: reqwest::RequestBuilder) -> Result<Value, HackMdError> {
        let response = request.bearer_auth(self.token).send().await?;
        let status = response.status();
        let bytes = response.bytes().await?;
        if status.is_success() {
            if bytes.is_empty() {
                return Ok(json!({ "ok": true }));
            }
            return serde_json::from_slice(&bytes).map_err(|error| {
                HackMdError::Api(format!("failed to decode HackMD JSON response: {error}"))
            });
        }

        let body = String::from_utf8_lossy(&bytes);
        Err(HackMdError::Api(format!(
            "status {} from HackMD: {}",
            status.as_u16(),
            body
        )))
    }

    fn url(&self, path: &str) -> Result<Url, HackMdError> {
        Ok(self.base_url.join(path.trim_start_matches('/'))?)
    }
}

pub(crate) async fn verify_token(
    client: &reqwest::Client,
    hackmd_api_url: &str,
    hackmd_api_token: &str,
) -> Result<(), HackMdError> {
    let api = HackMdClient::new(client, hackmd_api_url, hackmd_api_token)?;
    api.get_me().await.map(|_| ())
}

fn json_body(value: Value) -> Value {
    if value.is_null() { json!({}) } else { value }
}
