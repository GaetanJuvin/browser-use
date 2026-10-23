use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};
pub const HOST_NAME: &str = "com.gaetanjuvin.browser_use";
pub fn manifest(extension_id: &str, binary: &Path) -> Result<Value> {
    if extension_id.len() != 32 || !extension_id.bytes().all(|b| (b'a'..=b'p').contains(&b)) {
        bail!("extension ID must contain exactly 32 letters a–p; copy it from chrome://extensions");
    }
    if !binary.is_absolute() {
        bail!("native host executable path must be absolute");
    }
    Ok(
        json!({"name":HOST_NAME,"description":"Browser Use Rust native host","path":binary,"type":"stdio","allowed_origins":[format!("chrome-extension://{extension_id}/")]}),
    )
}
pub fn default_manifest_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("cannot determine home directory")?;
    #[cfg(target_os = "macos")]
    let folder = home.join("Library/Application Support/Google/Chrome/NativeMessagingHosts");
    #[cfg(not(target_os = "macos"))]
    let folder = home.join(".config/google-chrome/NativeMessagingHosts");
    Ok(folder.join(format!("{HOST_NAME}.json")))
}
pub fn install(extension_id: &str, output: Option<PathBuf>) -> Result<PathBuf> {
    let binary = std::env::current_exe()?.canonicalize()?;
    let value = manifest(extension_id, &binary)?;
    let path = output.map(Ok).unwrap_or_else(default_manifest_path)?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serde_json::to_vec_pretty(&value)?)?;
    Ok(path)
}
