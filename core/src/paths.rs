use std::path::{Path, PathBuf};

/// Data directory: `$CLIPSTASH_DIR`, or `~/Library/Application Support/clipstash`.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CLIPSTASH_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join("Library/Application Support/clipstash")
}

/// Create the directory (0700) if needed.
pub(crate) fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    set_mode(dir, 0o700);
    Ok(())
}

/// Restrict a file to its owner. Best-effort.
pub(crate) fn set_mode(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
}

/// Make a database file and its WAL side files user-readable only (0600).
pub(crate) fn protect_db(path: &Path) {
    set_mode(path, 0o600);
    for suffix in ["-wal", "-shm"] {
        let mut s = path.as_os_str().to_owned();
        s.push(suffix);
        let p = PathBuf::from(s);
        if p.exists() {
            set_mode(&p, 0o600);
        }
    }
}

/// Replace a leading home directory with `~` for display.
pub fn abbreviate_home(path: &str) -> String {
    if let Some(home) = std::env::var_os("HOME").and_then(|h| h.into_string().ok()) {
        if !home.is_empty() {
            if let Some(rest) = path.strip_prefix(&home) {
                if rest.is_empty() || rest.starts_with('/') {
                    return format!("~{rest}");
                }
            }
        }
    }
    path.to_string()
}
