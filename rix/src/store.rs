//! Store path parsing and validation (Nix semantics).

use crate::nix32;
use std::fmt;
use std::path::{Path, PathBuf};

pub const STORE_DIR: &str = "/nix/store";
pub const HASH_LEN: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorePath {
    pub hash: String,
    pub name: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum StorePathError {
    NotInStore,
    BadHash,
    BadName(String),
}

impl fmt::Display for StorePathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorePathError::NotInStore => write!(f, "not a path in {STORE_DIR}"),
            StorePathError::BadHash => write!(f, "invalid store path hash"),
            StorePathError::BadName(n) => write!(f, "invalid store path name '{n}'"),
        }
    }
}

/// Nix store path names: 1–211 chars of `[A-Za-z0-9+\-._?=]`, not starting with '.'.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 211
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+-._?=".contains(&b))
}

impl StorePath {
    /// Parse `/nix/store/<hash>-<name>` (or a path below it: the store path prefix is used).
    pub fn parse(p: &str) -> Result<StorePath, StorePathError> {
        let rest = p
            .strip_prefix(STORE_DIR)
            .and_then(|r| r.strip_prefix('/'))
            .ok_or(StorePathError::NotInStore)?;
        let base = rest.split('/').next().unwrap_or("");
        if base.len() < HASH_LEN + 2 || base.as_bytes()[HASH_LEN] != b'-' {
            return Err(StorePathError::BadHash);
        }
        let (hash, name) = (&base[..HASH_LEN], &base[HASH_LEN + 1..]);
        if !hash.bytes().all(nix32::is_valid_char) || nix32::decode(hash).is_none() {
            return Err(StorePathError::BadHash);
        }
        if !valid_name(name) {
            return Err(StorePathError::BadName(name.into()));
        }
        Ok(StorePath {
            hash: hash.into(),
            name: name.into(),
        })
    }

    pub fn path(&self) -> PathBuf {
        Path::new(STORE_DIR).join(format!("{}-{}", self.hash, self.name))
    }

    /// Split "firefox-128.0.3" into ("firefox", "128.0.3") using Nix's rule: the version
    /// starts at the first '-' followed by a non-letter.
    pub fn name_version(&self) -> (&str, &str) {
        split_name_version(&self.name)
    }
}

pub fn split_name_version(name: &str) -> (&str, &str) {
    let b = name.as_bytes();
    for i in 0..b.len() {
        if b[i] == b'-' && i + 1 < b.len() && !b[i + 1].is_ascii_alphabetic() {
            return (&name[..i], &name[i + 1..]);
        }
    }
    (name, "")
}

impl fmt::Display for StorePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}-{}", STORE_DIR, self.hash, self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_paths() {
        let p =
            StorePath::parse("/nix/store/0mdqa9w1p6cmli6976v4wi0sw9r4p5pr-hello-2.12.1/bin/hello")
                .unwrap();
        assert_eq!(p.name, "hello-2.12.1");
        assert_eq!(p.name_version(), ("hello", "2.12.1"));
        assert_eq!(
            p.to_string(),
            "/nix/store/0mdqa9w1p6cmli6976v4wi0sw9r4p5pr-hello-2.12.1"
        );
    }

    #[test]
    fn rejects_invalid_paths() {
        assert_eq!(
            StorePath::parse("/usr/bin/hello"),
            Err(StorePathError::NotInStore)
        );
        assert_eq!(
            StorePath::parse("/nix/store/short-x"),
            Err(StorePathError::BadHash)
        );
        // 'e' is not in the nix32 alphabet
        assert_eq!(
            StorePath::parse("/nix/store/emdqa9w1p6cmli6976v4wi0sw9r4p5pr-x"),
            Err(StorePathError::BadHash)
        );
        assert!(matches!(
            StorePath::parse("/nix/store/0mdqa9w1p6cmli6976v4wi0sw9r4p5pr-.hidden"),
            Err(StorePathError::BadName(_))
        ));
        assert!(matches!(
            StorePath::parse("/nix/store/0mdqa9w1p6cmli6976v4wi0sw9r4p5pr-a b"),
            Err(StorePathError::BadName(_))
        ));
    }

    #[test]
    fn name_version_split() {
        assert_eq!(split_name_version("gtk+3-3.24.43"), ("gtk+3", "3.24.43"));
        assert_eq!(
            split_name_version("xorg-server-21.1"),
            ("xorg-server", "21.1")
        );
        assert_eq!(split_name_version("nix-shell"), ("nix-shell", ""));
    }
}
