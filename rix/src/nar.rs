//! Native NAR (Nix ARchive) serialisation and hashing.
//!
//! `nar_hash(path)` produces the same SHA-256 as `nix hash path` / the `narHash`
//! recorded for store paths, without invoking Nix. The archive is streamed into the
//! hasher; nothing is buffered beyond one read chunk.

use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

struct NarWriter<W: Write> {
    out: W,
    written: u64,
}

impl<W: Write> NarWriter<W> {
    fn raw(&mut self, b: &[u8]) -> io::Result<()> {
        self.written += b.len() as u64;
        self.out.write_all(b)
    }

    fn pad(&mut self, len: u64) -> io::Result<()> {
        let rem = (len % 8) as usize;
        if rem != 0 {
            self.raw(&[0u8; 8][..8 - rem])?;
        }
        Ok(())
    }

    fn str(&mut self, s: &[u8]) -> io::Result<()> {
        self.raw(&(s.len() as u64).to_le_bytes())?;
        self.raw(s)?;
        self.pad(s.len() as u64)
    }

    fn node(&mut self, path: &Path) -> io::Result<()> {
        let meta = fs::symlink_metadata(path)?;
        self.str(b"(")?;
        let ft = meta.file_type();
        if ft.is_symlink() {
            self.str(b"type")?;
            self.str(b"symlink")?;
            self.str(b"target")?;
            self.str(fs::read_link(path)?.as_os_str().as_bytes())?;
        } else if ft.is_file() {
            self.str(b"type")?;
            self.str(b"regular")?;
            if meta.permissions().mode() & 0o100 != 0 {
                self.str(b"executable")?;
                self.str(b"")?;
            }
            self.str(b"contents")?;
            let len = meta.len();
            self.raw(&len.to_le_bytes())?;
            let mut f = fs::File::open(path)?;
            let mut buf = vec![0u8; 256 * 1024];
            let mut copied = 0u64;
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                self.raw(&buf[..n])?;
                copied += n as u64;
            }
            if copied != len {
                return Err(io::Error::other(format!(
                    "{}: file changed while reading",
                    path.display()
                )));
            }
            self.pad(len)?;
        } else if ft.is_dir() {
            self.str(b"type")?;
            self.str(b"directory")?;
            let mut entries: Vec<_> = fs::read_dir(path)?.collect::<Result<_, _>>()?;
            entries.sort_by(|a, b| a.file_name().as_bytes().cmp(b.file_name().as_bytes()));
            for e in entries {
                self.str(b"entry")?;
                self.str(b"(")?;
                self.str(b"name")?;
                self.str(e.file_name().as_bytes())?;
                self.str(b"node")?;
                self.node(&e.path())?;
                self.str(b")")?;
            }
        } else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("{}: unsupported file type", path.display()),
            ));
        }
        self.str(b")")
    }
}

/// Serialise `path` as a NAR into `out`. Returns the archive size in bytes.
pub fn dump(path: &Path, out: impl Write) -> io::Result<u64> {
    let mut w = NarWriter { out, written: 0 };
    w.str(b"nix-archive-1")?;
    w.node(path)?;
    Ok(w.written)
}

struct HashSink(Sha256);

impl Write for HashSink {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.update(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// SHA-256 of the NAR serialisation of `path`, plus the NAR size.
pub fn nar_hash(path: &Path) -> io::Result<([u8; 32], u64)> {
    let mut sink = HashSink(Sha256::new());
    let size = dump(path, &mut sink)?;
    Ok((sink.0.finalize().into(), size))
}

/// Flat SHA-256 of a file's contents (`nix hash file`).
pub fn file_hash(path: &Path) -> io::Result<[u8; 32]> {
    let mut f = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().into())
}

/// Format as Nix SRI (`sha256-<base64>`).
pub fn sri(hash: &[u8; 32]) -> String {
    format!("sha256-{}", base64(hash))
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let s = s.trim_end_matches('=').as_bytes();
    let mut out = Vec::new();
    for chunk in s.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= val(c)? << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrip() {
        for d in [
            &b""[..],
            b"a",
            b"ab",
            b"abc",
            b"abcd",
            &[0xff, 0x00, 0x7f, 0x80],
        ] {
            assert_eq!(base64_decode(&base64(d)).unwrap(), d);
        }
        assert_eq!(base64(b"abc"), "YWJj");
    }

    #[test]
    fn regular_file_nar_layout() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f");
        fs::write(&f, b"hello").unwrap();
        let mut buf = Vec::new();
        let n = dump(&f, &mut buf).unwrap();
        assert_eq!(n as usize, buf.len());
        assert_eq!(buf.len() % 8, 0);
        assert_eq!(&buf[8..21], b"nix-archive-1");
    }
}
