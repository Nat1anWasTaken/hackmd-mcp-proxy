use super::models::Workspace;

pub(crate) fn notes_collection_path(workspace: &Workspace) -> String {
    match workspace {
        Workspace::Personal => "notes".to_owned(),
        Workspace::Team { team_path } => format!("teams/{}/notes", encode_path_segment(team_path)),
    }
}

pub(crate) fn note_item_path(workspace: &Workspace, note_id: &str) -> String {
    match workspace {
        Workspace::Personal => format!("notes/{}", encode_path_segment(note_id)),
        Workspace::Team { team_path } => format!(
            "teams/{}/notes/{}",
            encode_path_segment(team_path),
            encode_path_segment(note_id)
        ),
    }
}

pub(crate) fn folders_collection_path(workspace: &Workspace) -> String {
    match workspace {
        Workspace::Personal => "folders".to_owned(),
        Workspace::Team { team_path } => {
            format!("teams/{}/folders", encode_path_segment(team_path))
        }
    }
}

pub(crate) fn folder_item_path(workspace: &Workspace, folder_id: &str) -> String {
    match workspace {
        Workspace::Personal => format!("folders/{}", encode_path_segment(folder_id)),
        Workspace::Team { team_path } => format!(
            "teams/{}/folders/{}",
            encode_path_segment(team_path),
            encode_path_segment(folder_id)
        ),
    }
}

pub(crate) fn patch_path(workspace: &Workspace, note_id: &str) -> String {
    match workspace {
        Workspace::Personal => format!("notes/{note_id}.md"),
        Workspace::Team { team_path } => format!("teams/{team_path}/notes/{note_id}.md"),
    }
}

fn encode_path_segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::{Workspace, notes_collection_path};

    #[test]
    fn team_notes_route_uses_team_endpoint() {
        assert_eq!(
            notes_collection_path(&Workspace::Team {
                team_path: "core-team".to_owned()
            }),
            "teams/core-team/notes"
        );
    }
}
