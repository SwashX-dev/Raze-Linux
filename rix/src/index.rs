//! Package search index.
//!
//! `nix search` evaluates nixpkgs on every query (tens of seconds and gigabytes of RAM
//! on a cold cache). Rix evaluates once per nixpkgs revision (delegated to Nix), stores
//! a compact tab-separated index keyed by the revision's narHash, and answers queries
//! natively by scanning it. Results come from the same evaluation as `nix search`, so
//! they are identical; only the query path is different.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub attr: String,
    pub pname: String,
    pub version: String,
    pub description: String,
}

pub struct Index {
    pub key: String,
    pub entries: Vec<Entry>,
}

const HEADER: &str = "#rix-index v1";

fn clean(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

impl Index {
    /// Build from the JSON emitted by `nix search --json <flake> ^`.
    pub fn from_nix_search_json(key: &str, json: &str) -> Result<Index, String> {
        let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let obj = v.as_object().ok_or("expected a JSON object")?;
        let mut entries: Vec<Entry> = obj
            .iter()
            .map(|(attr, e)| {
                let attr = attr
                    .splitn(3, '.')
                    .nth(2)
                    .filter(|_| {
                        attr.starts_with("legacyPackages.") || attr.starts_with("packages.")
                    })
                    .unwrap_or(attr);
                Entry {
                    attr: clean(attr),
                    pname: clean(e["pname"].as_str().unwrap_or("")),
                    version: clean(e["version"].as_str().unwrap_or("")),
                    description: clean(e["description"].as_str().unwrap_or("")),
                }
            })
            .collect();
        entries.sort_by(|a, b| a.attr.cmp(&b.attr));
        Ok(Index {
            key: key.into(),
            entries,
        })
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(d) = path.parent() {
            fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension("tmp");
        let mut f = io::BufWriter::new(fs::File::create(&tmp)?);
        writeln!(f, "{HEADER} {}", self.key)?;
        for e in &self.entries {
            writeln!(
                f,
                "{}\t{}\t{}\t{}",
                e.attr, e.pname, e.version, e.description
            )?;
        }
        f.flush()?;
        drop(f);
        fs::rename(tmp, path)
    }

    pub fn load(path: &Path) -> io::Result<Index> {
        let text = fs::read_to_string(path)?;
        let mut lines = text.lines();
        let header = lines.next().unwrap_or("");
        let key = header
            .strip_prefix(HEADER)
            .map(|k| k.trim().to_string())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not a rix index"))?;
        let entries = lines
            .filter_map(|l| {
                let mut it = l.splitn(4, '\t');
                Some(Entry {
                    attr: it.next()?.to_string(),
                    pname: it.next()?.to_string(),
                    version: it.next()?.to_string(),
                    description: it.next().unwrap_or("").to_string(),
                })
            })
            .collect();
        Ok(Index { key, entries })
    }

    pub fn get(&self, attr: &str) -> Option<&Entry> {
        self.entries
            .binary_search_by(|e| e.attr.as_str().cmp(attr))
            .ok()
            .map(|i| &self.entries[i])
    }

    /// All query terms must match (case-insensitively) in the attribute path, package
    /// name or description. Results are ranked: exact name > prefix > substring >
    /// description match.
    pub fn search(&self, query: &[String], limit: usize) -> Vec<&Entry> {
        let terms: Vec<String> = query.iter().map(|t| t.to_lowercase()).collect();
        if terms.is_empty() {
            return vec![];
        }
        let mut scored: Vec<(u32, &Entry)> = Vec::new();
        for e in &self.entries {
            let attr = e.attr.to_lowercase();
            let pname = e.pname.to_lowercase();
            let leaf = attr.rsplit('.').next().unwrap_or(&attr).to_string();
            let mut total = 0u32;
            let mut all = true;
            for t in &terms {
                let s = if leaf == *t || attr == *t {
                    1000
                } else if pname == *t {
                    900
                } else if leaf.starts_with(t.as_str()) {
                    500
                } else if pname.contains(t.as_str()) {
                    300
                } else if attr.contains(t.as_str()) {
                    200
                } else if e.description.to_lowercase().contains(t.as_str()) {
                    50
                } else {
                    0
                };
                if s == 0 {
                    all = false;
                    break;
                }
                total += s;
            }
            if all {
                // Prefer top-level packages over nested sets (python3Packages.foo).
                let depth_penalty = attr.matches('.').count() as u32 * 40;
                scored.push((total.saturating_sub(depth_penalty), e));
            }
        }
        scored.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then(a.1.attr.len().cmp(&b.1.attr.len()))
                .then(a.1.attr.cmp(&b.1.attr))
        });
        scored.into_iter().take(limit).map(|(_, e)| e).collect()
    }
}

pub fn cache_dir() -> PathBuf {
    if let Ok(d) = std::env::var("RIX_CACHE_DIR") {
        return d.into();
    }
    if let Ok(d) = std::env::var("XDG_CACHE_HOME") {
        return Path::new(&d).join("rix");
    }
    if let Ok(h) = std::env::var("HOME") {
        return Path::new(&h).join(".cache/rix");
    }
    PathBuf::from("/var/cache/rix")
}

pub fn index_path(key: &str) -> PathBuf {
    let safe: String = key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    cache_dir().join(format!("index-{safe}.tsv"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Index {
        let json = r#"{
          "legacyPackages.x86_64-linux.firefox": {"pname":"firefox","version":"144.0","description":"Web browser built from Firefox source tree"},
          "legacyPackages.x86_64-linux.firefox-esr": {"pname":"firefox-esr","version":"140.4","description":"Web browser (extended support release)"},
          "legacyPackages.x86_64-linux.python3Packages.firefox-tools": {"pname":"firefox-tools","version":"1.0","description":"Tools"},
          "legacyPackages.x86_64-linux.librewolf": {"pname":"librewolf","version":"144.0","description":"Fork of Firefox, focused on privacy"},
          "legacyPackages.x86_64-linux.hello": {"pname":"hello","version":"2.12.2","description":"Program that produces a familiar, friendly greeting"}
        }"#;
        Index::from_nix_search_json("sha256-test", json).unwrap()
    }

    #[test]
    fn ranks_exact_matches_first() {
        let idx = sample();
        let r: Vec<&str> = idx
            .search(&["firefox".into()], 10)
            .iter()
            .map(|e| e.attr.as_str())
            .collect();
        assert_eq!(r[0], "firefox");
        assert_eq!(r[1], "firefox-esr");
        assert_eq!(*r.last().unwrap(), "librewolf"); // description-only match ranks last
        assert!(r.contains(&"python3Packages.firefox-tools"));
    }

    #[test]
    fn all_terms_must_match() {
        let idx = sample();
        let r = idx.search(&["web".into(), "extended".into()], 10);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].attr, "firefox-esr");
        assert!(idx.search(&["nonexistent".into()], 10).is_empty());
    }

    #[test]
    fn save_load_roundtrip_and_lookup() {
        let d = tempfile::tempdir().unwrap();
        let idx = sample();
        let p = d.path().join("i.tsv");
        idx.save(&p).unwrap();
        let back = Index::load(&p).unwrap();
        assert_eq!(back.key, "sha256-test");
        assert_eq!(back.entries, idx.entries);
        assert_eq!(back.get("hello").unwrap().version, "2.12.2");
        assert!(back.get("nope").is_none());
    }
}
