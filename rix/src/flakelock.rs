//! Native reader for flake.lock (version 7).

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedInput {
    pub name: String,
    pub kind: String,
    pub source: String,
    pub rev: Option<String>,
    pub nar_hash: Option<String>,
    pub last_modified: Option<i64>,
}

pub fn parse(text: &str) -> Result<Vec<LockedInput>, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let version = v["version"].as_u64().unwrap_or(0);
    if !(5..=7).contains(&version) {
        return Err(format!("unsupported flake.lock version {version}"));
    }
    let nodes = v["nodes"].as_object().ok_or("flake.lock has no nodes")?;
    let root = v["root"].as_str().unwrap_or("root");
    let inputs = nodes
        .get(root)
        .and_then(|r| r["inputs"].as_object())
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    let sorted: BTreeMap<_, _> = inputs.into_iter().collect();
    for (name, target) in sorted {
        // A follows-path (array) points through other nodes; only report direct locks.
        let Some(node_name) = target.as_str() else {
            continue;
        };
        let Some(node) = nodes.get(node_name) else {
            continue;
        };
        let l = &node["locked"];
        let kind = l["type"].as_str().unwrap_or("?").to_string();
        let source = match kind.as_str() {
            "github" | "gitlab" | "sourcehut" => format!(
                "{}:{}/{}",
                kind,
                l["owner"].as_str().unwrap_or("?"),
                l["repo"].as_str().unwrap_or("?")
            ),
            "path" => l["path"].as_str().unwrap_or("?").to_string(),
            _ => l["url"].as_str().unwrap_or("?").to_string(),
        };
        out.push(LockedInput {
            name,
            kind,
            source,
            rev: l["rev"].as_str().map(String::from),
            nar_hash: l["narHash"].as_str().map(String::from),
            last_modified: l["lastModified"].as_i64(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_root_inputs() {
        let lock = r#"{"nodes":{"nixpkgs":{"locked":{"lastModified":1762000000,"narHash":"sha256-abc=","owner":"NixOS","repo":"nixpkgs","rev":"1d060086d60cb76a40cf37109f979d88a5bd6396","type":"github"},"original":{}},
          "root":{"inputs":{"nixpkgs":"nixpkgs","other":["nixpkgs"]}}},"root":"root","version":7}"#;
        let i = super::parse(lock).unwrap();
        assert_eq!(i.len(), 1);
        assert_eq!(i[0].source, "github:NixOS/nixpkgs");
        assert_eq!(
            i[0].rev.as_deref(),
            Some("1d060086d60cb76a40cf37109f979d88a5bd6396")
        );
        assert!(super::parse(r#"{"version":1,"nodes":{}}"#).is_err());
    }
}
