use std::cmp::Reverse;

use axum::{
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::patch;

const PROTOCOL_VERSION: &str = "2025-06-18";
const DEFAULT_LIST_LIMIT: usize = 50;
const MAX_LIST_LIMIT: usize = 200;

#[derive(Debug, thiserror::Error)]
pub enum HackMdError {
    #[error("HackMD API token is not configured for this user")]
    MissingCredential,
    #[error("invalid HackMD API URL")]
    InvalidApiUrl(#[from] url::ParseError),
    #[error("invalid HackMD API request: {0}")]
    InvalidRequest(String),
    #[error("HackMD API request failed: {0}")]
    Api(String),
    #[error("HackMD API upstream request failed")]
    Upstream(#[from] reqwest::Error),
    #[error(transparent)]
    Patch(#[from] patch::PatchError),
}

impl IntoResponse for HackMdError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::MissingCredential => StatusCode::FORBIDDEN,
            Self::InvalidRequest(_) | Self::Patch(_) => StatusCode::BAD_REQUEST,
            Self::InvalidApiUrl(_) | Self::Api(_) | Self::Upstream(_) => StatusCode::BAD_GATEWAY,
        };
        tracing::warn!(error = %self, "HackMD request failed");
        (status, self.to_string()).into_response()
    }
}

pub async fn verify_token(
    client: &reqwest::Client,
    hackmd_api_url: &str,
    hackmd_api_token: &str,
) -> Result<(), HackMdError> {
    let api = HackMdClient::new(client, hackmd_api_url, hackmd_api_token)?;
    api.get_me().await.map(|_| ())
}

pub async fn handle_mcp_request(
    client: &reqwest::Client,
    hackmd_api_url: &str,
    hackmd_api_token: &str,
    request: JsonRpcRequest,
) -> JsonRpcResponse {
    let api = match HackMdClient::new(client, hackmd_api_url, hackmd_api_token) {
        Ok(api) => api,
        Err(error) => return request.error(-32603, error.to_string()),
    };

    match request.method.as_str() {
        "initialize" => request.result(initialize_result()),
        "notifications/initialized" => request.result(json!({})),
        "tools/list" => request.result(json!({ "tools": tool_definitions() })),
        "tools/call" => {
            let params = match request.params.clone() {
                Some(params) => params,
                None => return request.error(-32602, "tools/call params are required"),
            };
            match call_tool(&api, params).await {
                Ok(result) => request.result(result),
                Err(error) => request.error(-32000, error.to_string()),
            }
        }
        _ => request.error(-32601, "method not found"),
    }
}

async fn call_tool(api: &HackMdClient<'_>, params: Value) -> Result<Value, HackMdError> {
    let request: ToolCallRequest = serde_json::from_value(params)
        .map_err(|error| HackMdError::InvalidRequest(error.to_string()))?;
    let arguments = request.arguments.unwrap_or_else(|| json!({}));
    let result = match request.name.as_str() {
        "hackmd_list_notes" => {
            let input: ListNotesInput = parse_args(arguments)?;
            json!(api.list_notes(input).await?)
        }
        "hackmd_get_note" => {
            let input: NoteRefInput = parse_args(arguments)?;
            json!(api.get_note(input).await?)
        }
        "hackmd_create_note" => {
            let input: CreateNoteInput = parse_args(arguments)?;
            api.create_note(input).await?
        }
        "hackmd_edit_note" => {
            let input: EditNoteInput = parse_args(arguments)?;
            json!(api.edit_note(input).await?)
        }
        "hackmd_update_note" => {
            let input: UpdateNoteInput = parse_args(arguments)?;
            api.update_note(input).await?
        }
        "hackmd_delete_note" => {
            let input: NoteRefInput = parse_args(arguments)?;
            api.delete_note(input).await?
        }
        "hackmd_list_folders" => {
            let input: WorkspaceInput = parse_args(arguments)?;
            api.list_folders(input.workspace).await?
        }
        "hackmd_create_folder" => {
            let input: CreateFolderInput = parse_args(arguments)?;
            api.create_folder(input).await?
        }
        "hackmd_update_folder" => {
            let input: UpdateFolderInput = parse_args(arguments)?;
            api.update_folder(input).await?
        }
        "hackmd_delete_folder" => {
            let input: FolderRefInput = parse_args(arguments)?;
            api.delete_folder(input).await?
        }
        _ => return Err(HackMdError::InvalidRequest("unknown tool".to_owned())),
    };
    Ok(tool_result(result))
}

fn parse_args<T>(arguments: Value) -> Result<T, HackMdError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(arguments)
        .map_err(|error| HackMdError::InvalidRequest(error.to_string()))
}

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    #[serde(default)]
    pub id: Option<Value>,
    #[allow(dead_code)]
    pub jsonrpc: Option<String>,
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

impl JsonRpcRequest {
    fn result(&self, result: Value) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0",
            id: self.id.clone(),
            result: Some(result),
            error: None,
        }
    }

    fn error(&self, code: i64, message: impl Into<String>) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0",
            id: self.id.clone(),
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ToolCallRequest {
    name: String,
    arguments: Option<Value>,
}

#[derive(Debug)]
struct HackMdClient<'a> {
    client: &'a reqwest::Client,
    base_url: Url,
    token: &'a str,
}

impl<'a> HackMdClient<'a> {
    fn new(
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

    async fn list_notes(&self, input: ListNotesInput) -> Result<ListNotesOutput, HackMdError> {
        let workspace = input.workspace.unwrap_or_default();
        let mut notes: Vec<NoteSummary> = self
            .get_json(&notes_collection_path(&workspace))
            .await?
            .into_iter()
            .map(|value| note_summary(&workspace, value))
            .collect();

        if let Some(query) = input
            .query
            .as_deref()
            .map(normalize_search)
            .filter(|q| !q.is_empty())
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
            NoteSort::TitleAsc => notes.sort_by_key(|note| note.title.to_lowercase()),
        }

        let total = notes.len();
        let offset = input.offset.unwrap_or(0).min(total);
        let limit = input
            .limit
            .unwrap_or(DEFAULT_LIST_LIMIT)
            .min(MAX_LIST_LIMIT);
        let notes = notes.into_iter().skip(offset).take(limit).collect();

        Ok(ListNotesOutput {
            total,
            offset,
            limit,
            notes,
        })
    }

    async fn get_note(&self, input: NoteRefInput) -> Result<NoteOutput, HackMdError> {
        let note = self
            .get(&note_item_path(&input.workspace, &input.note_id))
            .await?;
        Ok(note_output(&input.workspace, &input.note_id, note))
    }

    async fn create_note(&self, input: CreateNoteInput) -> Result<Value, HackMdError> {
        let path = notes_collection_path(&input.workspace);
        self.post(&path, json_body(input.note)).await
    }

    async fn edit_note(&self, input: EditNoteInput) -> Result<EditNoteOutput, HackMdError> {
        let note = self
            .get(&note_item_path(&input.workspace, &input.note_id))
            .await?;
        let content = note_content(&note)?;
        let patch_path = patch_path(&input.workspace, &input.note_id);
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

    async fn update_note(&self, input: UpdateNoteInput) -> Result<Value, HackMdError> {
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

    async fn delete_note(&self, input: NoteRefInput) -> Result<Value, HackMdError> {
        self.delete(&note_item_path(&input.workspace, &input.note_id))
            .await
    }

    async fn list_folders(&self, workspace: Workspace) -> Result<Value, HackMdError> {
        self.get(&folders_collection_path(&workspace)).await
    }

    async fn create_folder(&self, input: CreateFolderInput) -> Result<Value, HackMdError> {
        self.post(&folders_collection_path(&input.workspace), input.folder)
            .await
    }

    async fn update_folder(&self, input: UpdateFolderInput) -> Result<Value, HackMdError> {
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

    async fn delete_folder(&self, input: FolderRefInput) -> Result<Value, HackMdError> {
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

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Workspace {
    Personal,
    Team { team_path: String },
}

impl Default for Workspace {
    fn default() -> Self {
        Self::Personal
    }
}

#[derive(Debug, Deserialize)]
struct WorkspaceInput {
    #[serde(default)]
    workspace: Workspace,
}

#[derive(Debug, Deserialize)]
struct ListNotesInput {
    #[serde(default)]
    workspace: Option<Workspace>,
    query: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    folder_id: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
    sort: Option<NoteSort>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum NoteSort {
    LastChangedDesc,
    LastChangedAsc,
    CreatedDesc,
    CreatedAsc,
    TitleAsc,
}

impl Default for NoteSort {
    fn default() -> Self {
        Self::LastChangedDesc
    }
}

#[derive(Debug, Deserialize)]
struct NoteRefInput {
    #[serde(default)]
    workspace: Workspace,
    note_id: String,
}

#[derive(Debug, Deserialize)]
struct CreateNoteInput {
    #[serde(default)]
    workspace: Workspace,
    #[serde(default)]
    note: Value,
}

#[derive(Debug, Deserialize)]
struct EditNoteInput {
    #[serde(default)]
    workspace: Workspace,
    note_id: String,
    patch: String,
}

#[derive(Debug, Deserialize)]
struct UpdateNoteInput {
    #[serde(default)]
    workspace: Workspace,
    note_id: String,
    fields: Value,
}

#[derive(Debug, Deserialize)]
struct CreateFolderInput {
    #[serde(default)]
    workspace: Workspace,
    #[serde(default)]
    folder: Value,
}

#[derive(Debug, Deserialize)]
struct UpdateFolderInput {
    #[serde(default)]
    workspace: Workspace,
    folder_id: String,
    fields: Value,
}

#[derive(Debug, Deserialize)]
struct FolderRefInput {
    #[serde(default)]
    workspace: Workspace,
    folder_id: String,
}

#[derive(Debug, Serialize)]
struct ListNotesOutput {
    total: usize,
    offset: usize,
    limit: usize,
    notes: Vec<NoteSummary>,
}

#[derive(Debug, Serialize)]
struct NoteSummary {
    id: String,
    short_id: Option<String>,
    title: String,
    description: String,
    tags: Vec<String>,
    workspace: Workspace,
    patch_path: String,
    folder_ids: Vec<String>,
    created_at: SortableNumber,
    last_changed_at: SortableNumber,
    publish_link: Option<String>,
    permalink: Option<String>,
}

impl NoteSummary {
    fn matches_query(&self, query: &str) -> bool {
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
struct SortableNumber(i64);

#[derive(Debug, Serialize)]
struct NoteOutput {
    note_id: String,
    patch_path: String,
    content: Option<String>,
    note: Value,
}

#[derive(Debug, Serialize)]
struct EditNoteOutput {
    note_id: String,
    patch_path: String,
    changed: bool,
    content: String,
}

fn note_summary(workspace: &Workspace, note: Value) -> NoteSummary {
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

fn note_output(workspace: &Workspace, note_id: &str, note: Value) -> NoteOutput {
    NoteOutput {
        note_id: note_id.to_owned(),
        patch_path: patch_path(workspace, note_id),
        content: note_content(&note).ok(),
        note,
    }
}

fn note_content(note: &Value) -> Result<String, HackMdError> {
    ["content", "text", "markdown"]
        .iter()
        .find_map(|field| note.get(*field)?.as_str().map(ToOwned::to_owned))
        .ok_or_else(|| {
            HackMdError::Api("HackMD note response did not include editable content".to_owned())
        })
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

fn normalize_search(value: &str) -> String {
    value.trim().to_lowercase()
}

fn notes_collection_path(workspace: &Workspace) -> String {
    match workspace {
        Workspace::Personal => "notes".to_owned(),
        Workspace::Team { team_path } => format!("teams/{}/notes", encode_path_segment(team_path)),
    }
}

fn note_item_path(workspace: &Workspace, note_id: &str) -> String {
    match workspace {
        Workspace::Personal => format!("notes/{}", encode_path_segment(note_id)),
        Workspace::Team { team_path } => format!(
            "teams/{}/notes/{}",
            encode_path_segment(team_path),
            encode_path_segment(note_id)
        ),
    }
}

fn folders_collection_path(workspace: &Workspace) -> String {
    match workspace {
        Workspace::Personal => "folders".to_owned(),
        Workspace::Team { team_path } => {
            format!("teams/{}/folders", encode_path_segment(team_path))
        }
    }
}

fn folder_item_path(workspace: &Workspace, folder_id: &str) -> String {
    match workspace {
        Workspace::Personal => format!("folders/{}", encode_path_segment(folder_id)),
        Workspace::Team { team_path } => format!(
            "teams/{}/folders/{}",
            encode_path_segment(team_path),
            encode_path_segment(folder_id)
        ),
    }
}

fn patch_path(workspace: &Workspace, note_id: &str) -> String {
    match workspace {
        Workspace::Personal => format!("notes/{note_id}.md"),
        Workspace::Team { team_path } => format!("teams/{team_path}/notes/{note_id}.md"),
    }
}

fn encode_path_segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn json_body(value: Value) -> Value {
    if value.is_null() { json!({}) } else { value }
}

fn tool_result(value: Value) -> Value {
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
    json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": value,
        "isError": false
    })
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "hackmd-mcp-proxy", "version": "0.1.0" }
    })
}

fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "hackmd_list_notes",
            "description": "List HackMD notes from a personal or team workspace. Supports proxy-side metadata search with query over title, description, tags, id, and shortId; this does not search note body content.",
            "inputSchema": {
                "type": "object",
                "properties": common_list_properties(),
                "additionalProperties": false
            }
        }),
        json!({
            "name": "hackmd_get_note",
            "description": "Get one HackMD note with full content, metadata, and the exact patch_path to use with hackmd_edit_note.",
            "inputSchema": note_ref_schema()
        }),
        json!({
            "name": "hackmd_create_note",
            "description": "Create a HackMD note in a personal or team workspace.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspace": workspace_schema(),
                    "note": {
                        "type": "object",
                        "description": "HackMD note fields such as title, content, tags, description, readPermission, writePermission, permalink, and parentFolderId."
                    }
                },
                "required": ["note"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "hackmd_edit_note",
            "description": "Default tool for normal HackMD content edits. Prefer this over hackmd_update_note for editing note bodies. Provide a Codex-style patch from hackmd_get_note.patch_path; the server applies it only when context matches.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspace": workspace_schema(),
                    "note_id": { "type": "string" },
                    "patch": { "type": "string", "description": "Codex-style patch block with *** Begin Patch, *** Update File: <patch_path>, hunks, and *** End Patch." }
                },
                "required": ["note_id", "patch"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "hackmd_update_note",
            "description": "Fallback/full HackMD note update tool. Do not use for normal content edits unless hackmd_edit_note failed or is insufficient. Use this when changing metadata such as title, tags, permissions, description, permalink, or parentFolderId, or when a full content replacement is explicitly necessary.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspace": workspace_schema(),
                    "note_id": { "type": "string" },
                    "fields": {
                        "type": "object",
                        "description": "HackMD PATCH fields. May include metadata fields and, as fallback only, content."
                    }
                },
                "required": ["note_id", "fields"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "hackmd_delete_note",
            "description": "Delete a HackMD note from a personal or team workspace.",
            "inputSchema": note_ref_schema()
        }),
        json!({
            "name": "hackmd_list_folders",
            "description": "List folders in a personal or team HackMD workspace.",
            "inputSchema": workspace_input_schema()
        }),
        json!({
            "name": "hackmd_create_folder",
            "description": "Create a folder in a personal or team HackMD workspace.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspace": workspace_schema(),
                    "folder": { "type": "object", "description": "HackMD folder fields such as name, description, icon, color, and parentFolderId." }
                },
                "required": ["folder"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "hackmd_update_folder",
            "description": "Update folder metadata in a personal or team HackMD workspace.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspace": workspace_schema(),
                    "folder_id": { "type": "string" },
                    "fields": { "type": "object", "description": "HackMD folder PATCH fields." }
                },
                "required": ["folder_id", "fields"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "hackmd_delete_folder",
            "description": "Delete a folder from a personal or team HackMD workspace.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspace": workspace_schema(),
                    "folder_id": { "type": "string" }
                },
                "required": ["folder_id"],
                "additionalProperties": false
            }
        }),
    ]
}

fn common_list_properties() -> Value {
    json!({
        "workspace": workspace_schema(),
        "query": { "type": "string", "description": "Case-insensitive metadata search over title, description, tags, id, and shortId." },
        "tags": { "type": "array", "items": { "type": "string" }, "description": "Require every provided tag." },
        "folder_id": { "type": "string" },
        "limit": { "type": "integer", "minimum": 1, "maximum": MAX_LIST_LIMIT, "default": DEFAULT_LIST_LIMIT },
        "offset": { "type": "integer", "minimum": 0, "default": 0 },
        "sort": {
            "type": "string",
            "enum": ["last_changed_desc", "last_changed_asc", "created_desc", "created_asc", "title_asc"],
            "default": "last_changed_desc"
        }
    })
}

fn workspace_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "workspace": workspace_schema()
        },
        "additionalProperties": false
    })
}

fn note_ref_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "workspace": workspace_schema(),
            "note_id": { "type": "string" }
        },
        "required": ["note_id"],
        "additionalProperties": false
    })
}

fn workspace_schema() -> Value {
    json!({
        "oneOf": [
            {
                "type": "object",
                "properties": { "kind": { "const": "personal" } },
                "required": ["kind"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "kind": { "const": "team" },
                    "team_path": { "type": "string" }
                },
                "required": ["kind", "team_path"],
                "additionalProperties": false
            }
        ],
        "default": { "kind": "personal" }
    })
}

pub fn bearer_challenge(resource_metadata_url: &str) -> Response {
    let mut response = StatusCode::UNAUTHORIZED.into_response();
    if let Ok(value) = HeaderValue::from_str(&format!(
        r#"Bearer resource_metadata="{resource_metadata_url}""#
    )) {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ListNotesInput, Workspace, note_summary, notes_collection_path};

    #[test]
    fn team_notes_route_uses_team_endpoint() {
        assert_eq!(
            notes_collection_path(&Workspace::Team {
                team_path: "core-team".to_owned()
            }),
            "teams/core-team/notes"
        );
    }

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
