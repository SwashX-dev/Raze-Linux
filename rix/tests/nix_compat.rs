//! Cross-checks native hashing against the real `nix` binary when it is available.

use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;
use std::process::Command;

fn nix_hash_path(p: &Path) -> Option<String> {
    let out = Command::new("nix")
        .args([
            "--extra-experimental-features",
            "nix-command",
            "--store",
            "dummy://",
            "hash",
            "path",
            "--base32",
        ])
        .arg(p)
        .output()
        .ok()?;
    if !out.status.success() {
        eprintln!(
            "skipping: nix unavailable: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        return None;
    }
    Some(String::from_utf8(out.stdout).ok()?.trim().to_string())
}

#[test]
fn nar_hash_matches_nix_for_a_mixed_tree() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("tree");
    std::fs::create_dir_all(root.join("sub/deeper")).unwrap();
    std::fs::write(root.join("a.txt"), b"hello\n").unwrap();
    std::fs::write(root.join("empty"), b"").unwrap();
    std::fs::write(root.join("sub/odd-size.bin"), vec![7u8; 1234567]).unwrap();
    std::fs::write(root.join("sub/deeper/run.sh"), b"#!/bin/sh\necho hi\n").unwrap();
    std::fs::set_permissions(
        root.join("sub/deeper/run.sh"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    symlink("../a.txt", root.join("sub/link")).unwrap();
    std::fs::write(root.join("B-upper"), b"sorting is bytewise").unwrap();

    let (h, _) = rix::nar::nar_hash(&root).unwrap();
    let native = rix::nix32::encode(&h);
    if let Some(expected) = nix_hash_path(&root) {
        assert_eq!(native, expected);
    }
    // Single files and symlinks at the root are valid NAR roots too.
    let (hf, _) = rix::nar::nar_hash(&root.join("a.txt")).unwrap();
    if let Some(expected) = nix_hash_path(&root.join("a.txt")) {
        assert_eq!(rix::nix32::encode(&hf), expected);
    }
}
