//! The frontend's own small settings file: which daemon it last connected to.
//!
//! Lives outside the space (that belongs to the daemon) in the user's config
//! dir: %APPDATA%\commander\frontend.json on Windows, $XDG_CONFIG_HOME (or
//! ~/.config)/commander/frontend.json elsewhere.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_API: &str = "http://127.0.0.1:7700";

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Settings {
    /// daemon base url, e.g. http://192.168.2.7:7700
    #[serde(default)]
    pub api: String,
}

fn path() -> Option<PathBuf> {
    let dir = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }?;
    Some(dir.join("commander").join("frontend.json"))
}

pub fn load() -> Settings {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(s: &Settings) {
    let Some(p) = path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_string_pretty(s) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&p, json) {
                eprintln!("could not save {}: {}", p.display(), e);
            }
        }
        Err(e) => eprintln!("could not encode settings: {}", e),
    }
}

/// turn what the user typed into a base url: "host", "host:port",
/// "http://host:port/" all become "http://host:port"
pub fn normalize(input: &str) -> String {
    let s = input.trim().trim_end_matches('/');
    let s = if s.contains("://") { s.to_string() } else { format!("http://{}", s) };
    let (scheme, rest) = s.split_once("://").unwrap_or(("http", s.as_str()));
    let rest = if rest.is_empty() { "127.0.0.1" } else { rest };
    // IPv6 literal without a port, e.g. [::1]
    let has_port = if rest.starts_with('[') {
        rest.rsplit_once(']').map_or(false, |(_, tail)| tail.starts_with(':'))
    } else {
        rest.contains(':')
    };
    if has_port {
        format!("{}://{}", scheme, rest)
    } else {
        format!("{}://{}:7700", scheme, rest)
    }
}

/// whether the url points at this machine, so a missing daemon could be
/// started here
pub fn is_local(base: &str) -> bool {
    let host = base.split("://").nth(1).unwrap_or(base);
    let host = host.trim_start_matches('[');
    let host = host.split(|c| c == ']' || c == ':' || c == '/').next().unwrap_or("");
    matches!(host, "127.0.0.1" | "localhost" | "::1" | "0.0.0.0")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalizes() {
        assert_eq!(normalize("192.168.2.7"), "http://192.168.2.7:7700");
        assert_eq!(normalize("192.168.2.7:8000/"), "http://192.168.2.7:8000");
        assert_eq!(normalize("http://x:1"), "http://x:1");
        assert_eq!(normalize(""), "http://127.0.0.1:7700");
        assert_eq!(normalize("[::1]"), "http://[::1]:7700");
        assert!(is_local("http://localhost:7700"));
        assert!(!is_local("http://192.168.2.7:7700"));
    }
}
