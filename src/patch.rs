#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum PatchError {
    #[error("patch must start with *** Begin Patch")]
    MissingBegin,
    #[error("patch must end with *** End Patch")]
    MissingEnd,
    #[error("patch must contain exactly one *** Update File section")]
    InvalidUpdateCount,
    #[error("patch targets {actual}, expected {expected}")]
    WrongTarget { actual: String, expected: String },
    #[error("patch operation is not supported by hackmd_edit_note: {0}")]
    UnsupportedOperation(String),
    #[error("patch hunk must start with @@")]
    MissingHunk,
    #[error("patch hunk context was not found")]
    ContextNotFound,
    #[error("patch hunk context matched multiple locations")]
    AmbiguousContext,
    #[error("patch line must start with space, +, or -")]
    InvalidLine,
}

#[derive(Debug)]
struct FilePatch {
    target: String,
    hunks: Vec<Hunk>,
}

#[derive(Debug)]
struct Hunk {
    lines: Vec<HunkLine>,
}

#[derive(Debug)]
enum HunkLine {
    Context(String),
    Add(String),
    Remove(String),
}

pub fn apply_note_patch(
    content: &str,
    patch: &str,
    expected_target: &str,
) -> Result<String, PatchError> {
    let file_patch = parse_patch(patch)?;
    if file_patch.target != expected_target {
        return Err(PatchError::WrongTarget {
            actual: file_patch.target,
            expected: expected_target.to_owned(),
        });
    }

    let trailing_newline = content.ends_with('\n');
    let mut lines = content
        .strip_suffix('\n')
        .unwrap_or(content)
        .split('\n')
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if content.is_empty() {
        lines.clear();
    }

    for hunk in file_patch.hunks {
        apply_hunk(&mut lines, hunk)?;
    }

    let mut output = lines.join("\n");
    if trailing_newline {
        output.push('\n');
    }
    Ok(output)
}

fn parse_patch(patch: &str) -> Result<FilePatch, PatchError> {
    let lines = patch.lines().collect::<Vec<_>>();
    if lines.first() != Some(&"*** Begin Patch") {
        return Err(PatchError::MissingBegin);
    }
    if lines.last() != Some(&"*** End Patch") {
        return Err(PatchError::MissingEnd);
    }

    let mut target = None;
    let mut hunks = Vec::new();
    let mut current_hunk: Option<Hunk> = None;

    for line in lines.iter().skip(1).take(lines.len().saturating_sub(2)) {
        if let Some(path) = line.strip_prefix("*** Update File: ") {
            if target.replace(path.to_owned()).is_some() {
                return Err(PatchError::InvalidUpdateCount);
            }
            continue;
        }
        if line.starts_with("*** Add File: ")
            || line.starts_with("*** Delete File: ")
            || line.starts_with("*** Move to: ")
        {
            return Err(PatchError::UnsupportedOperation((*line).to_owned()));
        }
        if *line == "@@" || line.starts_with("@@ ") {
            if let Some(hunk) = current_hunk.take() {
                hunks.push(hunk);
            }
            current_hunk = Some(Hunk { lines: Vec::new() });
            continue;
        }

        let Some(hunk) = current_hunk.as_mut() else {
            return Err(PatchError::MissingHunk);
        };
        if let Some(text) = line.strip_prefix(' ') {
            hunk.lines.push(HunkLine::Context(text.to_owned()));
        } else if let Some(text) = line.strip_prefix('+') {
            hunk.lines.push(HunkLine::Add(text.to_owned()));
        } else if let Some(text) = line.strip_prefix('-') {
            hunk.lines.push(HunkLine::Remove(text.to_owned()));
        } else {
            return Err(PatchError::InvalidLine);
        }
    }

    if let Some(hunk) = current_hunk {
        hunks.push(hunk);
    }

    if target.is_none() || hunks.is_empty() {
        return Err(PatchError::InvalidUpdateCount);
    }

    Ok(FilePatch {
        target: target.expect("checked above"),
        hunks,
    })
}

fn apply_hunk(lines: &mut Vec<String>, hunk: Hunk) -> Result<(), PatchError> {
    let old = hunk
        .lines
        .iter()
        .filter_map(|line| match line {
            HunkLine::Context(value) | HunkLine::Remove(value) => Some(value.clone()),
            HunkLine::Add(_) => None,
        })
        .collect::<Vec<_>>();
    let new = hunk
        .lines
        .iter()
        .filter_map(|line| match line {
            HunkLine::Context(value) | HunkLine::Add(value) => Some(value.clone()),
            HunkLine::Remove(_) => None,
        })
        .collect::<Vec<_>>();

    let position = find_unique_match(lines, &old)?;
    lines.splice(position..position + old.len(), new);
    Ok(())
}

fn find_unique_match(lines: &[String], needle: &[String]) -> Result<usize, PatchError> {
    if needle.is_empty() {
        return Err(PatchError::ContextNotFound);
    }
    let mut matches = lines
        .windows(needle.len())
        .enumerate()
        .filter_map(|(index, window)| (window == needle).then_some(index));

    let Some(first) = matches.next() else {
        return Err(PatchError::ContextNotFound);
    };
    if matches.next().is_some() {
        return Err(PatchError::AmbiguousContext);
    }
    Ok(first)
}

#[cfg(test)]
mod tests {
    use super::{PatchError, apply_note_patch};

    #[test]
    fn applies_update_hunk() -> Result<(), PatchError> {
        let patch = "*** Begin Patch\n*** Update File: notes/a.md\n@@\n title\n-old\n+new\n end\n*** End Patch";

        let output = apply_note_patch("title\nold\nend\n", patch, "notes/a.md")?;

        assert_eq!(output, "title\nnew\nend\n");
        Ok(())
    }

    #[test]
    fn rejects_wrong_target() {
        let patch = "*** Begin Patch\n*** Update File: notes/b.md\n@@\n-old\n+new\n*** End Patch";

        assert_eq!(
            apply_note_patch("old", patch, "notes/a.md"),
            Err(PatchError::WrongTarget {
                actual: "notes/b.md".to_owned(),
                expected: "notes/a.md".to_owned()
            })
        );
    }

    #[test]
    fn rejects_missing_context() {
        let patch = "*** Begin Patch\n*** Update File: notes/a.md\n@@\n-old\n+new\n*** End Patch";

        assert_eq!(
            apply_note_patch("other", patch, "notes/a.md"),
            Err(PatchError::ContextNotFound)
        );
    }

    #[test]
    fn rejects_ambiguous_context() {
        let patch = "*** Begin Patch\n*** Update File: notes/a.md\n@@\n-old\n+new\n*** End Patch";

        assert_eq!(
            apply_note_patch("old\nold", patch, "notes/a.md"),
            Err(PatchError::AmbiguousContext)
        );
    }

    #[test]
    fn rejects_add_file_operation() {
        let patch = "*** Begin Patch\n*** Add File: notes/a.md\n+new\n*** End Patch";

        assert!(matches!(
            apply_note_patch("", patch, "notes/a.md"),
            Err(PatchError::UnsupportedOperation(_))
        ));
    }

    #[test]
    fn applies_multiple_hunks() -> Result<(), PatchError> {
        let patch = "*** Begin Patch\n*** Update File: notes/a.md\n@@\n-one\n+1\n@@\n-three\n+3\n*** End Patch";

        let output = apply_note_patch("one\ntwo\nthree\n", patch, "notes/a.md")?;

        assert_eq!(output, "1\ntwo\n3\n");
        Ok(())
    }

    #[test]
    fn applies_delete_only_hunk() -> Result<(), PatchError> {
        let patch =
            "*** Begin Patch\n*** Update File: notes/a.md\n@@\n keep\n-remove\n end\n*** End Patch";

        let output = apply_note_patch("keep\nremove\nend\n", patch, "notes/a.md")?;

        assert_eq!(output, "keep\nend\n");
        Ok(())
    }

    #[test]
    fn preserves_missing_trailing_newline() -> Result<(), PatchError> {
        let patch = "*** Begin Patch\n*** Update File: notes/a.md\n@@\n-old\n+new\n*** End Patch";

        let output = apply_note_patch("old", patch, "notes/a.md")?;

        assert_eq!(output, "new");
        Ok(())
    }

    #[test]
    fn rejects_missing_begin() {
        let patch = "*** Update File: notes/a.md\n@@\n-old\n+new\n*** End Patch";

        assert_eq!(
            apply_note_patch("old", patch, "notes/a.md"),
            Err(PatchError::MissingBegin)
        );
    }

    #[test]
    fn rejects_missing_end() {
        let patch = "*** Begin Patch\n*** Update File: notes/a.md\n@@\n-old\n+new";

        assert_eq!(
            apply_note_patch("old", patch, "notes/a.md"),
            Err(PatchError::MissingEnd)
        );
    }

    #[test]
    fn rejects_duplicate_update_sections() {
        let patch = "*** Begin Patch\n*** Update File: notes/a.md\n*** Update File: notes/a.md\n@@\n-old\n+new\n*** End Patch";

        assert_eq!(
            apply_note_patch("old", patch, "notes/a.md"),
            Err(PatchError::InvalidUpdateCount)
        );
    }

    #[test]
    fn rejects_missing_hunk() {
        let patch = "*** Begin Patch\n*** Update File: notes/a.md\n-old\n+new\n*** End Patch";

        assert_eq!(
            apply_note_patch("old", patch, "notes/a.md"),
            Err(PatchError::MissingHunk)
        );
    }

    #[test]
    fn rejects_invalid_line() {
        let patch = "*** Begin Patch\n*** Update File: notes/a.md\n@@\nold\n*** End Patch";

        assert_eq!(
            apply_note_patch("old", patch, "notes/a.md"),
            Err(PatchError::InvalidLine)
        );
    }
}
