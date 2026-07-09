use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    client::HackMdClient,
    error::HackMdError,
    models::{
        CreateFolderInput, CreateNoteInput, EditNoteInput, FolderRefInput, ListNotesInput,
        NoteRefInput, UpdateFolderInput, UpdateNoteInput, WorkspaceInput,
    },
    schema::{initialize_result, tool_definitions},
};

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
            api.get_note(input).await?
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

fn tool_result(value: Value) -> Value {
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
    json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": value,
        "isError": false
    })
}
