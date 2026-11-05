//! r6-log: read lines on stdin and append them, timestamped, to a size-rotated log
//! directory. Usage: r6-log [-s MAX_BYTES] [-n KEEP] DIR

use std::io::Read;

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut max, mut keep, mut dir) = (512 * 1024u64, 3u32, None);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-s" => max = args.next().and_then(|v| v.parse().ok()).unwrap_or(max),
            "-n" => keep = args.next().and_then(|v| v.parse().ok()).unwrap_or(keep),
            "-h" | "--help" => {
                println!("usage: r6-log [-s MAX_BYTES] [-n KEEP] DIR");
                return;
            }
            _ => dir = Some(a),
        }
    }
    let Some(dir) = dir else {
        eprintln!("usage: r6-log [-s MAX_BYTES] [-n KEEP] DIR");
        std::process::exit(2);
    };
    let mut w = r6::log::LogWriter::new(std::path::Path::new(&dir), max, keep);
    let mut stdin = std::io::stdin().lock();
    let mut buf = [0u8; 8192];
    loop {
        match stdin.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if let Err(e) = w.write(&buf[..n]) {
                    eprintln!("r6-log: {dir}: {e}");
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    let _ = w.flush_partial();
}
