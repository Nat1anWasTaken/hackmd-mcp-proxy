use std::collections::BTreeSet;

use serde::Serialize;
use thiserror::Error;

pub const HACKMD_READ: &str = "hackmd.read";
pub const HACKMD_WRITE: &str = "hackmd.write";
pub const HACKMD_DELETE: &str = "hackmd.delete";

pub const SUPPORTED_SCOPES: [&str; 3] = [HACKMD_READ, HACKMD_WRITE, HACKMD_DELETE];

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScopeSet {
    scopes: BTreeSet<String>,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ScopeError {
    #[error("unsupported OAuth scope: {0}")]
    Unsupported(String),
}

impl ScopeSet {
    pub fn parse(raw: Option<&str>) -> Result<Self, ScopeError> {
        let raw = raw.unwrap_or(HACKMD_READ);
        let mut scopes = BTreeSet::new();
        for scope in raw.split_whitespace().filter(|scope| !scope.is_empty()) {
            if !SUPPORTED_SCOPES.contains(&scope) {
                return Err(ScopeError::Unsupported(scope.to_owned()));
            }
            scopes.insert(scope.to_owned());
        }

        if scopes.is_empty() {
            scopes.insert(HACKMD_READ.to_owned());
        }

        Ok(Self { scopes })
    }

    pub fn as_space_delimited(&self) -> String {
        self.scopes.iter().cloned().collect::<Vec<_>>().join(" ")
    }

    pub fn contains(&self, scope: &str) -> bool {
        self.scopes.contains(scope)
    }
}

#[cfg(test)]
mod tests {
    use super::{ScopeError, ScopeSet, HACKMD_READ, HACKMD_WRITE};

    #[test]
    fn parses_supported_scope_set() -> anyhow::Result<()> {
        let scopes = ScopeSet::parse(Some("hackmd.write hackmd.read"))?;

        assert_eq!(scopes.as_space_delimited(), "hackmd.read hackmd.write");
        assert!(scopes.contains(HACKMD_READ));
        assert!(scopes.contains(HACKMD_WRITE));
        Ok(())
    }

    #[test]
    fn rejects_unknown_scope() {
        assert_eq!(
            ScopeSet::parse(Some("offline_access")),
            Err(ScopeError::Unsupported("offline_access".to_owned()))
        );
    }
}
