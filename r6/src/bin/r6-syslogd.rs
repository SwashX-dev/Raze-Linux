//! r6-syslogd: a minimal syslog(3) receiver for r6 systems.
//!
//! Listens on /dev/log (datagram socket) and writes one line per message to stdout,
//! where the r6 supervisor's log collector stores it (`r6 logs syslog`). Without a
//! /dev/log socket, programs using syslog(3) with LOG_CONS write to the console.
//! Usage: r6-syslogd [SOCKET]   (default /dev/log)

use std::io::Write;
use std::os::unix::net::UnixDatagram;

const SEVERITIES: [&str; 8] = [
    "emerg", "alert", "crit", "err", "warning", "notice", "info", "debug",
];
const FACILITIES: [&str; 24] = [
    "kern", "user", "mail", "daemon", "auth", "syslog", "lpr", "news", "uucp", "cron", "authpriv",
    "ftp", "ntp", "audit", "alert", "clock", "local0", "local1", "local2", "local3", "local4",
    "local5", "local6", "local7",
];

/// Turn a raw syslog datagram ("<PRI>Mmm dd hh:mm:ss tag[pid]: msg") into
/// "facility.severity tag[pid]: msg". The timestamp is dropped (the log collector adds its own).
pub fn format(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let text = text.trim_end_matches(['\n', '\0']);
    let (pri, rest) = match text.strip_prefix('<').and_then(|t| t.split_once('>')) {
        Some((p, r)) => match p.parse::<usize>() {
            Ok(p) => (Some(p), r),
            Err(_) => (None, text),
        },
        None => (None, text),
    };
    // RFC 3164 timestamp: "Nov  5 13:25:22 " (16 bytes).
    let rest = if rest.len() > 16
        && rest.as_bytes()[3] == b' '
        && rest.as_bytes()[6] == b' '
        && rest.as_bytes()[9] == b':'
    {
        &rest[16..]
    } else {
        rest
    };
    let rest = rest.replace('\n', " ");
    match pri {
        Some(p) => format!(
            "{}.{} {}",
            FACILITIES.get(p >> 3).copied().unwrap_or("unknown"),
            SEVERITIES[p & 7],
            rest
        ),
        None => rest,
    }
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "/dev/log".into());
    let _ = std::fs::remove_file(&path);
    let sock = match UnixDatagram::bind(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("r6-syslogd: cannot bind {path}: {e}");
            std::process::exit(1);
        }
    };
    // Everyone may log.
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666));
    let mut buf = vec![0u8; 64 * 1024];
    let stdout = std::io::stdout();
    loop {
        match sock.recv(&mut buf) {
            Ok(n) => {
                let mut out = stdout.lock();
                let _ = writeln!(out, "{}", format(&buf[..n]));
                let _ = out.flush();
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                eprintln!("r6-syslogd: {e}");
                std::process::exit(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::format;

    #[test]
    fn parses_priority_and_drops_timestamp() {
        assert_eq!(
            format(b"<29>Nov  5 13:25:22 ModemManager[577]: <msg> started\n"),
            "daemon.notice ModemManager[577]: <msg> started"
        );
        assert_eq!(
            format(b"<86>sudo: raze : TTY=pts/0"),
            "authpriv.info sudo: raze : TTY=pts/0"
        );
        assert_eq!(format(b"no priority"), "no priority");
    }
}
