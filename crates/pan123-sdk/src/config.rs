use std::env;
use std::fs;
use std::path::PathBuf;

use crate::error::Result;
use crate::models::{CwdStore, TokenStore};
use crate::secure_storage::SecureStorage;

pub const TOKEN_FILE: &str = "123pan_token.json";
pub const CWD_FILE: &str = "123pan_cwd.json";

pub fn load_token() -> Option<String> {
    let storage = SecureStorage::auto(default_state_path(TOKEN_FILE));

    if let Some(token) = storage.load_token() {
        return Some(token);
    }

    let path = resolve_existing_state_path(TOKEN_FILE)?;
    let text = fs::read_to_string(path).ok()?;
    let token_store = serde_json::from_str::<TokenStore>(&text).ok()?;
    Some(token_store.token)
}

pub fn save_token(token: &str) -> Result<()> {
    let storage = SecureStorage::auto(default_state_path(TOKEN_FILE));
    storage.save_token(token)?;

    let legacy_path = default_state_path(TOKEN_FILE);
    if legacy_path.exists() {
        let _ = fs::remove_file(legacy_path);
    }

    Ok(())
}

pub fn load_cwd() -> CwdStore {
    let Some(path) = resolve_existing_state_path(CWD_FILE) else {
        return CwdStore::default();
    };

    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<CwdStore>(&text).ok())
        .unwrap_or_default()
}

pub fn save_cwd(cwd: &CwdStore) -> Result<()> {
    fs::write(
        default_state_path(CWD_FILE),
        serde_json::to_string_pretty(cwd)?,
    )?;
    Ok(())
}

pub fn resume_meta_dir() -> PathBuf {
    let path = config_dir().join("resume");
    let _ = fs::create_dir_all(&path);
    path
}

pub fn resume_meta_path_for(target: &PathBuf) -> PathBuf {
    let digest = format!("{:x}", md5::compute(target.to_string_lossy().as_bytes()));
    resume_meta_dir().join(format!("{digest}.json"))
}

pub fn clear_resume_meta_dir() -> Result<usize> {
    let dir = resume_meta_dir();
    let mut removed = 0usize;
    if !dir.exists() {
        return Ok(0);
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            fs::remove_file(entry.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

fn default_state_path(file_name: &str) -> PathBuf {
    config_dir().join(file_name)
}

fn resolve_existing_state_path(file_name: &str) -> Option<PathBuf> {
    state_candidates(file_name)
        .into_iter()
        .find(|path| path.exists())
}

fn state_candidates(file_name: &str) -> Vec<PathBuf> {
    let mut candidates = vec![default_state_path(file_name)];

    if let Ok(current_dir) = env::current_dir() {
        candidates.push(
            current_dir
                .join("crates")
                .join("pan123-cli")
                .join(file_name),
        );
        candidates.push(
            current_dir
                .join("crates")
                .join("pan123-sdk")
                .join(file_name),
        );
    }

    dedup_paths(candidates)
}

fn config_dir() -> PathBuf {
    if let Ok(dir) = env::var("PAN123_CONFIG_DIR") {
        let path = PathBuf::from(dir);
        let _ = fs::create_dir_all(&path);
        return path;
    }

    let path = default_app_config_dir();
    let _ = fs::create_dir_all(&path);
    path
}

fn default_app_config_dir() -> PathBuf {
    if let Ok(appdata) = env::var("APPDATA") {
        return PathBuf::from(appdata).join("pan123-cli");
    }
    if let Ok(home) = env::var("USERPROFILE") {
        return PathBuf::from(home).join(".pan123-cli");
    }
    PathBuf::from(".pan123-cli")
}

fn dedup_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut unique = Vec::new();
    for path in paths {
        if !unique.iter().any(|existing| existing == &path) {
            unique.push(path);
        }
    }
    unique
}
