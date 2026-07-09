use serde_json::{Value, json};

use super::models::{DEFAULT_LIST_LIMIT, MAX_LIST_LIMIT};

const PROTOCOL_VERSION: &str = "2025-06-18";

pub(crate) fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "hackmd-mcp-proxy", "version": "0.1.0" }
    })
}

pub(crate) fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "hackmd_list_notes",
            "description": "List HackMD notes from a personal or team workspace. Supports proxy-side metadata search with query over title, description, tags, id, and shortId; this does not search note body content.",
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false
            },
            "inputSchema": {
                "type": "object",
                "properties": common_list_properties(),
                "additionalProperties": false
            }
        }),
        json!({
            "name": "hackmd_get_note",
            "description": "Get one HackMD note with full content, metadata, and the exact patch_path to use with hackmd_edit_note.",
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false
            },
            "inputSchema": note_ref_schema()
        }),
        json!({
            "name": "hackmd_create_note",
            "description": "Create a HackMD note in a personal or team workspace.",
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": false
            },
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
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": false
            },
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
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": false
            },
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
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": true
            },
            "inputSchema": note_ref_schema()
        }),
        json!({
            "name": "hackmd_list_folders",
            "description": "List folders in a personal or team HackMD workspace.",
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false
            },
            "inputSchema": workspace_input_schema()
        }),
        json!({
            "name": "hackmd_create_folder",
            "description": "Create a folder in a personal or team HackMD workspace.",
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": false
            },
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
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": false
            },
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
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": true
            },
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
