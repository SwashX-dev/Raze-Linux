//! Nix profiles and generations, read and switched natively.
//!
//! A profile is a symlink `<dir>/<name>` pointing at `<name>-<N>-link`, which points
//! into the store. Rolling back atomically repoints the profile symlink (write a
//! temporary symlink, then rename(2) over the old one) — the same on-disk operation
//! `nix-env --rollback` performs, so Nix and Rix stay interchangeable.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Generation {
    pub number: u64,
    pub link: PathBuf,
    pub target: PathBuf,
    pub created: Option<SystemTime>,
    pub current: bool,
}

pub struct Profile {
    pub path: PathBuf,
}

impl Profile {
    pub fn new(path: impl Into<PathBuf>) -> Profile {
        Profile { path: path.into() }
    }

    /// The per-user default profile, following the same lookup order as Nix.
    pub fn user_default() -> Profile {
        if let Ok(home) = std::env::var("HOME") {
            let link = Path::new(&home).join(".nix-profile");
            if let Ok(t) = fs::read_link(&link) {
                let t = if t.is_absolute() {
                    t
                } else {
                    link.parent().unwrap().join(t)
                };
                return Profile::new(t);
            }
            let state = std::env::var("XDG_STATE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| Path::new(&home).join(".local/state"));
            return Profile::new(state.join("nix/profiles/profile"));
        }
        Profile::new("/nix/var/nix/profiles/default")
    }

    pub fn system() -> Profile {
        Profile::new("/nix/var/nix/profiles/system")
    }

    fn dir_and_name(&self) -> (PathBuf, String) {
        let dir = self
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let name = self
            .path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        (dir, name)
    }

    pub fn current_number(&self) -> Option<u64> {
        let (_, name) = self.dir_and_name();
        let t = fs::read_link(&self.path).ok()?;
        parse_link_name(t.file_name()?.to_str()?, &name)
    }

    pub fn generations(&self) -> io::Result<Vec<Generation>> {
        let (dir, name) = self.dir_and_name();
        let current = self.current_number();
        let mut out = Vec::new();
        let rd = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e),
        };
        for e in rd.flatten() {
            let fname = e.file_name();
            let Some(fname) = fname.to_str() else {
                continue;
            };
            let Some(n) = parse_link_name(fname, &name) else {
                continue;
            };
            let link = e.path();
            let target = fs::read_link(&link).unwrap_or_default();
            let created = fs::symlink_metadata(&link).and_then(|m| m.modified()).ok();
            out.push(Generation {
                number: n,
                link,
                target,
                created,
                current: Some(n) == current,
            });
        }
        out.sort_by_key(|g| g.number);
        Ok(out)
    }

    /// Point the profile at generation `n` atomically.
    pub fn switch_to(&self, n: u64) -> io::Result<PathBuf> {
        let (dir, name) = self.dir_and_name();
        let link_name = format!("{name}-{n}-link");
        if fs::symlink_metadata(dir.join(&link_name)).is_err() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("generation {n} does not exist"),
            ));
        }
        let tmp = dir.join(format!(".{name}-rix-tmp-{}", std::process::id()));
        let _ = fs::remove_file(&tmp);
        std::os::unix::fs::symlink(&link_name, &tmp)?;
        if let Err(e) = fs::rename(&tmp, &self.path) {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        Ok(dir.join(link_name))
    }

    /// Switch to the newest generation older than the current one.
    pub fn rollback(&self) -> io::Result<u64> {
        let cur = self.current_number().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "profile has no current generation")
        })?;
        let prev = self
            .generations()?
            .into_iter()
            .filter(|g| g.number < cur)
            .map(|g| g.number)
            .max()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "no previous generation to roll back to",
                )
            })?;
        self.switch_to(prev)?;
        Ok(prev)
    }
}

fn parse_link_name(fname: &str, profile: &str) -> Option<u64> {
    fname
        .strip_prefix(profile)?
        .strip_prefix('-')?
        .strip_suffix("-link")?
        .parse()
        .ok()
}

/// One installed element of a `nix profile` manifest (version 2 or 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub name: String,
    pub attr_path: Option<String>,
    pub original_url: Option<String>,
    pub store_paths: Vec<String>,
}

pub fn read_manifest(profile_target: &Path) -> io::Result<Vec<Element>> {
    let text = match fs::read_to_string(profile_target.join("manifest.json")) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e),
    };
    parse_manifest(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn parse_manifest(text: &str) -> Result<Vec<Element>, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let version = v["version"].as_u64().unwrap_or(0);
    let mut out = Vec::new();
    let mut push = |name: String, e: &serde_json::Value| {
        out.push(Element {
            name,
            attr_path: e["attrPath"].as_str().map(String::from),
            original_url: e["originalUrl"].as_str().map(String::from),
            store_paths: e["storePaths"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
        });
    };
    match version {
        3 => {
            if let Some(m) = v["elements"].as_object() {
                for (name, e) in m {
                    push(name.clone(), e);
                }
            }
        }
        1 | 2 => {
            if let Some(a) = v["elements"].as_array() {
                for e in a {
                    let name = e["attrPath"]
                        .as_str()
                        .and_then(|p| p.rsplit('.').next())
                        .unwrap_or("?")
                        .to_string();
                    push(name, e);
                }
            }
        }
        other => return Err(format!("unsupported profile manifest version {other}")),
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn fake_profile(dir: &Path, gens: &[u64], current: u64) -> Profile {
        for g in gens {
            let target = dir.join(format!("store-{g}"));
            fs::create_dir_all(&target).unwrap();
            symlink(&target, dir.join(format!("profile-{g}-link"))).unwrap();
        }
        symlink(format!("profile-{current}-link"), dir.join("profile")).unwrap();
        Profile::new(dir.join("profile"))
    }

    #[test]
    fn lists_generations_in_order() {
        let d = tempfile::tempdir().unwrap();
        let p = fake_profile(d.path(), &[3, 1, 10, 2], 10);
        let g = p.generations().unwrap();
        assert_eq!(
            g.iter().map(|g| g.number).collect::<Vec<_>>(),
            vec![1, 2, 3, 10]
        );
        assert!(g.last().unwrap().current);
        assert_eq!(p.current_number(), Some(10));
    }

    #[test]
    fn rollback_and_switch() {
        let d = tempfile::tempdir().unwrap();
        let p = fake_profile(d.path(), &[1, 2, 5], 5);
        assert_eq!(p.rollback().unwrap(), 2);
        assert_eq!(p.current_number(), Some(2));
        assert_eq!(p.rollback().unwrap(), 1);
        assert!(p.rollback().is_err());
        p.switch_to(5).unwrap();
        assert_eq!(p.current_number(), Some(5));
        assert!(p.switch_to(4).is_err());
        // No temporary links left behind.
        assert_eq!(
            fs::read_dir(d.path())
                .unwrap()
                .filter(|e| e
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with('.'))
                .count(),
            0
        );
    }

    #[test]
    fn parses_v3_manifest() {
        let m = r#"{"version":3,"elements":{"hello":{"active":true,"attrPath":"legacyPackages.x86_64-linux.hello","originalUrl":"flake:nixpkgs","storePaths":["/nix/store/0mdqa9w1p6cmli6976v4wi0sw9r4p5pr-hello-2.12.1"]}}}"#;
        let e = parse_manifest(m).unwrap();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].name, "hello");
        assert_eq!(e[0].store_paths.len(), 1);
        assert!(parse_manifest(r#"{"version":99}"#).is_err());
    }
}
