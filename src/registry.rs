use anyhow::{Context, Result, bail};
use serde_json::json;
use std::{
    fs,
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, PermissionsExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
};

pub fn default_dir() -> PathBuf {
    // SAFETY: geteuid has no preconditions or side effects.
    PathBuf::from(format!("/tmp/browser-use-{}", unsafe { libc::geteuid() }))
}
pub fn ensure_private(dir: &Path) -> Result<()> {
    if !dir.exists() {
        fs::DirBuilder::new()
            .mode(0o700)
            .create(dir)
            .context("create socket directory (parent must exist)")?;
    }
    let meta = fs::symlink_metadata(dir)?;
    // Reject symlinks and foreign-owned paths before changing permissions.
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } {
        bail!("socket directory must be an owned, real directory");
    }
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    Ok(())
}
pub fn write_active(dir: &Path, socket: &Path) -> Result<()> {
    let temp = dir.join(format!(".active-{}.json", uuid::Uuid::new_v4()));
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&temp)?;
    file.write_all(&serde_json::to_vec(
        &json!({"socketPath":socket,"pid":std::process::id()}),
    )?)?;
    if let Err(e) = fs::rename(&temp, dir.join("active.json")) {
        let _ = fs::remove_file(temp);
        return Err(e.into());
    }
    Ok(())
}
pub fn active_path(dir: &Path) -> Option<PathBuf> {
    let v: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("active.json")).ok()?).ok()?;
    let path = PathBuf::from(v["socketPath"].as_str()?);
    // Registry records are hints, and must not redirect outside this directory.
    if path.parent() != Some(dir) {
        return None;
    }
    Some(path)
}
pub fn validate_private(dir: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(dir).with_context(|| {
        format!(
            "no safe Chrome connection in {}; run setup and reload the extension",
            dir.display()
        )
    })?;
    // SAFETY: geteuid has no preconditions.
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
        bail!("socket discovery directory must be an owned, private, real directory");
    }
    Ok(())
}
fn connect_owned(path: &Path) -> Result<UnixStream> {
    use std::os::unix::fs::FileTypeExt;
    let meta = fs::symlink_metadata(path)?;
    // SAFETY: geteuid has no preconditions.
    if !meta.file_type().is_socket() || meta.uid() != unsafe { libc::geteuid() } {
        bail!("socket must belong to the current user");
    }
    Ok(UnixStream::connect(path)?)
}
pub fn connect(dir: &Path) -> Result<UnixStream> {
    validate_private(dir)?;
    if let Some(path) = active_path(dir) {
        if let Ok(s) = connect_owned(&path) {
            return Ok(s);
        }
    }
    let mut candidates = fs::read_dir(dir)
        .with_context(|| {
            format!(
                "no Chrome connection in {}; load the extension and run browser-use setup",
                dir.display()
            )
        })?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|e| e == "sock"))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|e| std::cmp::Reverse(e.metadata().and_then(|m| m.modified()).ok()));
    for e in candidates {
        if let Ok(s) = connect_owned(&e.path()) {
            return Ok(s);
        }
    }
    bail!(
        "no live Chrome connection; enable the Browser Use extension, check setup and restart Chrome"
    )
}
pub fn cleanup(dir: &Path, socket: &Path) {
    let _ = fs::remove_file(socket);
    if active_path(dir).as_deref() == Some(socket) {
        let _ = fs::remove_file(dir.join("active.json"));
    }
}
