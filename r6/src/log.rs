//! Size-rotated, line-oriented log files (r6-log).
//!
//! Layout per service: `<dir>/current`, `<dir>/current.1`, … `<dir>/current.<keep>`.
//! Each line is prefixed with seconds since boot (`[   12.345678]`, the same clock as
//! the kernel log) so service output can be correlated with `dmesg` without a
//! wall clock, which may not be set early in boot.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub struct LogWriter {
    dir: PathBuf,
    file: Option<File>,
    size: u64,
    max_bytes: u64,
    keep: u32,
    /// Bytes of an incomplete trailing line carried to the next write.
    partial: Vec<u8>,
}

impl LogWriter {
    pub fn new(dir: &Path, max_bytes: u64, keep: u32) -> LogWriter {
        LogWriter {
            dir: dir.into(),
            file: None,
            size: 0,
            max_bytes: max_bytes.max(4096),
            keep,
            partial: Vec::new(),
        }
    }

    pub fn current_path(dir: &Path) -> PathBuf {
        dir.join("current")
    }

    fn open(&mut self) -> io::Result<()> {
        if self.file.is_some() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.dir)?;
        let path = Self::current_path(&self.dir);
        let f = OpenOptions::new().create(true).append(true).open(&path)?;
        self.size = f.metadata().map(|m| m.len()).unwrap_or(0);
        self.file = Some(f);
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file = None;
        let cur = Self::current_path(&self.dir);
        if self.keep == 0 {
            std::fs::remove_file(&cur).ok();
        } else {
            for i in (1..self.keep).rev() {
                let from = self.dir.join(format!("current.{i}"));
                let to = self.dir.join(format!("current.{}", i + 1));
                if from.exists() {
                    std::fs::rename(&from, &to)?;
                }
            }
            std::fs::rename(&cur, self.dir.join("current.1"))?;
        }
        self.size = 0;
        self.open()
    }

    /// Append raw output; complete lines are timestamped and written.
    pub fn write(&mut self, data: &[u8]) -> io::Result<()> {
        self.partial.extend_from_slice(data);
        let mut start = 0;
        let mut out = Vec::new();
        let stamp = format!("[{:>12.6}] ", crate::sys::boottime_ms() / 1000.0);
        while let Some(nl) = self.partial[start..].iter().position(|&b| b == b'\n') {
            out.extend_from_slice(stamp.as_bytes());
            out.extend_from_slice(&self.partial[start..start + nl + 1]);
            start += nl + 1;
        }
        // Bound memory for pathological output without newlines.
        if self.partial.len() - start > 8192 {
            out.extend_from_slice(stamp.as_bytes());
            out.extend_from_slice(&self.partial[start..]);
            out.push(b'\n');
            start = self.partial.len();
        }
        self.partial.drain(..start);
        if out.is_empty() {
            return Ok(());
        }
        self.open()?;
        if self.size + out.len() as u64 > self.max_bytes && self.size > 0 {
            self.rotate()?;
        }
        let f = self.file.as_mut().expect("opened");
        f.write_all(&out)?;
        self.size += out.len() as u64;
        Ok(())
    }

    /// Flush an incomplete trailing line (e.g. when the producer exits).
    pub fn flush_partial(&mut self) -> io::Result<()> {
        if !self.partial.is_empty() {
            self.write(b"\n")?;
        }
        Ok(())
    }

    /// Write a supervisor-generated line (e.g. "started", "exited with code 1").
    pub fn note(&mut self, msg: &str) {
        let _ = self.write(format!("r6: {msg}\n").as_bytes());
    }
}

/// Read the last `n` lines from a service log directory (rotated files included).
pub fn tail(dir: &Path, n: usize) -> io::Result<Vec<String>> {
    let mut files = vec![LogWriter::current_path(dir)];
    for i in 1..100 {
        let p = dir.join(format!("current.{i}"));
        if !p.exists() {
            break;
        }
        files.push(p);
    }
    let mut lines: Vec<String> = Vec::new();
    for f in files {
        if lines.len() >= n {
            break;
        }
        let text = match std::fs::read(&f) {
            Ok(t) => String::from_utf8_lossy(&t).into_owned(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        let mut chunk: Vec<String> = text.lines().map(String::from).collect();
        chunk.append(&mut lines);
        lines = chunk;
    }
    let skip = lines.len().saturating_sub(n);
    Ok(lines.split_off(skip))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_lines_and_rotates() {
        let d = tempfile::tempdir().unwrap();
        let mut w = LogWriter::new(d.path(), 4096, 2);
        for i in 0..400 {
            w.write(format!("line number {i:04}\n").as_bytes()).unwrap();
        }
        assert!(d.path().join("current").exists());
        assert!(d.path().join("current.1").exists());
        assert!(d.path().join("current.2").exists());
        assert!(!d.path().join("current.3").exists());
        let t = tail(d.path(), 3).unwrap();
        assert_eq!(t.len(), 3);
        assert!(t[2].ends_with("line number 0399"), "{t:?}");
        for f in ["current", "current.1"] {
            assert!(std::fs::metadata(d.path().join(f)).unwrap().len() <= 4096 + 64);
        }
    }

    #[test]
    fn partial_lines_are_joined() {
        let d = tempfile::tempdir().unwrap();
        let mut w = LogWriter::new(d.path(), 1 << 20, 1);
        w.write(b"hel").unwrap();
        w.write(b"lo\nwor").unwrap();
        w.flush_partial().unwrap();
        let t = tail(d.path(), 10).unwrap();
        assert_eq!(t.len(), 2);
        assert!(t[0].ends_with("] hello"));
        assert!(t[1].ends_with("] wor"));
    }
}
