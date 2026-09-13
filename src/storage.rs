use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::workspace::{AppState, STATE_VERSION};

#[derive(Debug)]
pub struct LoadResult {
    pub state: AppState,
    pub warning: Option<String>,
    pub can_save: bool,
}

pub fn default_state_path() -> PathBuf {
    if let Some(data_directory) = std::env::var_os("CODEX_AIR_DATA_DIR") {
        return PathBuf::from(data_directory).join("state.json");
    }

    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Codex Air")
        .join("state.json")
}

pub fn load(path: &Path) -> LoadResult {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return LoadResult {
                state: AppState::default(),
                warning: None,
                can_save: true,
            };
        }
        Err(error) => {
            return LoadResult {
                state: AppState::default(),
                warning: Some(format!(
                    "Could not read workspace state at {}: {}",
                    path.display(),
                    error
                )),
                can_save: false,
            };
        }
    };

    let parsed = serde_json::from_slice::<AppState>(&bytes);
    let mut state = match parsed {
        Ok(state) if state.version == STATE_VERSION => state,
        Ok(state) => {
            let reason = format!(
                "workspace state uses unsupported version {} (supported version is {})",
                state.version, STATE_VERSION
            );
            return recover_default(path, &bytes, reason);
        }
        Err(error) => {
            return recover_default(
                path,
                &bytes,
                format!("invalid workspace state JSON: {error}"),
            );
        }
    };

    let normalized = state.normalize();

    LoadResult {
        state,
        warning: normalized.then_some(
            "Workspace state contained invalid references or values and was normalized.".to_owned(),
        ),
        can_save: true,
    }
}

fn recover_default(path: &Path, bytes: &[u8], reason: String) -> LoadResult {
    let backup = backup_original(path, bytes);
    let can_save = backup.is_ok();
    let warning = match backup {
        Ok(backup_path) => format!(
            "Could not load workspace state at {}: {}. The original file was preserved as {}.",
            path.display(),
            reason,
            backup_path.display()
        ),
        Err(error) => format!(
            "Could not load workspace state at {}: {}. The original file could not be backed up: {}.",
            path.display(),
            reason,
            error
        ),
    };

    LoadResult {
        state: AppState::default(),
        warning: Some(warning),
        can_save,
    }
}

fn backup_original(path: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "state.json".to_owned());

    for suffix in 0..1000u32 {
        let candidate_name = if suffix == 0 {
            format!("{file_name}.bak")
        } else {
            format!("{file_name}.bak.{suffix}")
        };
        let candidate = path.with_file_name(candidate_name);
        let mut backup = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        if let Err(error) = backup.write_all(bytes).and_then(|()| backup.sync_all()) {
            drop(backup);
            let _ = fs::remove_file(&candidate);
            return Err(error);
        }
        return Ok(candidate);
    }

    Err(io::Error::other("could not find an unused backup filename"))
}

pub fn save(path: &Path, state: &AppState) -> Result<()> {
    let serialized = serde_json::to_vec_pretty(state).context("serialize workspace state")?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("create state directory {}", parent.display()))?;
    }

    let temp_path = temporary_path(path);
    let mut temp_file = create_temp_file(&temp_path)?;
    if let Err(error) = temp_file
        .write_all(&serialized)
        .and_then(|()| temp_file.sync_all())
    {
        drop(temp_file);
        let _ = fs::remove_file(&temp_path);
        return Err(error).with_context(|| format!("write workspace state {}", path.display()));
    }
    drop(temp_file);

    let replacement = replace_file(&temp_path, path);
    if replacement.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    replacement.with_context(|| format!("replace workspace state {}", path.display()))
}

fn temporary_path(path: &Path) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "state.json".to_owned());
    path.with_file_name(format!(".{file_name}.tmp.{}.{}", std::process::id(), stamp))
}

fn create_temp_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

fn replace_file(temp_path: &Path, path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    use windows::core::PCWSTR;

    let temp: Vec<u16> = temp_path.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        MoveFileExW(
            PCWSTR(temp.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
        .map_err(|error| io::Error::other(error.to_string()))
    }
}
