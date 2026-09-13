use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The state format version written by this application.
pub const STATE_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppState {
    #[serde(default = "default_state_version")]
    pub version: u32,
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
    #[serde(default)]
    pub active_workspace: Option<Uuid>,
    #[serde(default)]
    pub window: Option<WindowPlacement>,
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: f32,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            workspaces: Vec::new(),
            active_workspace: None,
            window: None,
            sidebar_width: default_sidebar_width(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Workspace {
    pub id: Uuid,
    pub name: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub roots: Vec<WorkspaceRoot>,
    #[serde(default)]
    pub default_root: Option<Uuid>,
}

impl Workspace {
    pub fn display_name(&self) -> String {
        if let Some(name) = &self.name {
            return name.clone();
        }

        self.roots
            .first()
            .and_then(|root| root.path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "Workspace".to_owned())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceRoot {
    pub id: Uuid,
    pub path: PathBuf,
}

impl WorkspaceRoot {
    pub fn display_path(&self) -> String {
        let path = self.path.to_string_lossy();
        if let Some(path) = path.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{path}");
        }
        path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WindowPlacement {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

fn default_state_version() -> u32 {
    STATE_VERSION
}

fn default_sidebar_width() -> f32 {
    240.0
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum DirectoryIdentity {
    Windows { volume_serial: u32, file_index: u64 },
}

fn canonical_directory(path: &Path) -> Result<PathBuf> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("cannot inspect workspace folder {}", path.display()))?;
    if !metadata.is_dir() {
        return Err(anyhow!(
            "workspace path is not a directory: {}",
            path.display()
        ));
    }

    fs::canonicalize(path)
        .with_context(|| format!("cannot resolve workspace folder {}", path.display()))
}

fn directory_identity(path: &Path) -> Result<DirectoryIdentity> {
    let canonical = canonical_directory(path)?;
    windows_directory_identity(&canonical)
}

fn windows_directory_identity(path: &Path) -> Result<DirectoryIdentity> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle, OPEN_EXISTING,
    };
    use windows::core::PCWSTR;

    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let handle = unsafe {
        CreateFileW(
            PCWSTR(path.as_ptr()),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }
    .map_err(|error| anyhow!("cannot open workspace folder identity: {error}"))?;

    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    let identity = unsafe { GetFileInformationByHandle(handle, &mut information) }
        .map(|()| DirectoryIdentity::Windows {
            volume_serial: information.dwVolumeSerialNumber,
            file_index: (u64::from(information.nFileIndexHigh) << 32)
                | u64::from(information.nFileIndexLow),
        })
        .map_err(|error| anyhow!("cannot read workspace folder identity: {error}"));

    let close_result = unsafe { CloseHandle(handle) };
    if let Err(error) = close_result {
        return Err(anyhow!(
            "cannot close workspace folder identity handle: {error}"
        ));
    }
    identity
}

fn root_matches(root: &WorkspaceRoot, identity: &DirectoryIdentity) -> bool {
    directory_identity(&root.path)
        .map(|root_identity| root_identity == identity.clone())
        .unwrap_or(false)
}

impl AppState {
    pub fn normalize(&mut self) -> bool {
        let mut changed = false;
        let mut workspace_ids = HashSet::new();
        let mut root_ids = HashSet::new();

        for workspace in &mut self.workspaces {
            if workspace.id.is_nil() || !workspace_ids.insert(workspace.id) {
                workspace.id = fresh_id(&mut workspace_ids);
                changed = true;
            }

            if workspace
                .name
                .as_ref()
                .is_some_and(|name| name.trim().is_empty())
            {
                workspace.name = None;
                changed = true;
            }

            for root in &mut workspace.roots {
                if root.id.is_nil() || !root_ids.insert(root.id) {
                    root.id = fresh_id(&mut root_ids);
                    changed = true;
                }
            }

            let valid_default = workspace
                .default_root
                .is_some_and(|root_id| workspace.roots.iter().any(|root| root.id == root_id));
            if !valid_default {
                let default_root = workspace.roots.first().map(|root| root.id);
                if workspace.default_root != default_root {
                    workspace.default_root = default_root;
                    changed = true;
                }
            }
        }

        if self
            .active_workspace
            .is_some_and(|workspace_id| !workspace_ids.contains(&workspace_id))
        {
            self.active_workspace = None;
            changed = true;
        }

        if !self.sidebar_width.is_finite() {
            self.sidebar_width = default_sidebar_width();
            changed = true;
        } else {
            let normalized = self.sidebar_width.clamp(200.0, 380.0);
            if self.sidebar_width != normalized {
                self.sidebar_width = normalized;
                changed = true;
            }
        }

        if let Some(window) = &mut self.window {
            changed |= normalize_window(window);
        }

        changed
    }

    pub fn active(&self) -> Option<&Workspace> {
        let id = self.active_workspace?;
        self.workspaces.iter().find(|workspace| workspace.id == id)
    }

    pub fn active_mut(&mut self) -> Option<&mut Workspace> {
        let id = self.active_workspace?;
        self.workspaces
            .iter_mut()
            .find(|workspace| workspace.id == id)
    }

    pub fn open_folder(&mut self, path: PathBuf) -> Result<Uuid> {
        let canonical_path = canonical_directory(&path)?;
        let identity = directory_identity(&canonical_path)?;

        if let Some(workspace) = self.workspaces.iter().find(|workspace| {
            workspace.roots.len() == 1 && root_matches(&workspace.roots[0], &identity)
        }) {
            let id = workspace.id;
            self.select_workspace(id);
            return Ok(id);
        }

        let root_id = Uuid::new_v4();
        let workspace_id = Uuid::new_v4();
        self.workspaces.insert(
            0,
            Workspace {
                id: workspace_id,
                name: None,
                pinned: false,
                archived: false,
                thread_id: None,
                roots: vec![WorkspaceRoot {
                    id: root_id,
                    path: canonical_path,
                }],
                default_root: Some(root_id),
            },
        );
        self.active_workspace = Some(workspace_id);
        Ok(workspace_id)
    }

    pub fn add_root(&mut self, path: PathBuf) -> Result<bool> {
        let canonical_path = canonical_directory(&path)?;
        let identity = directory_identity(&canonical_path)?;
        let workspace = self
            .active_mut()
            .ok_or_else(|| anyhow!("cannot add a root without an active workspace"))?;

        if workspace
            .roots
            .iter()
            .any(|root| root_matches(root, &identity))
        {
            return Ok(false);
        }

        let root_id = Uuid::new_v4();
        if workspace.default_root.is_none() {
            workspace.default_root = Some(root_id);
        }
        workspace.roots.push(WorkspaceRoot {
            id: root_id,
            path: canonical_path,
        });
        Ok(true)
    }

    pub fn set_default_root(&mut self, root_id: Uuid) {
        if let Some(workspace) = self.active_mut()
            && workspace.roots.iter().any(|root| root.id == root_id)
        {
            workspace.default_root = Some(root_id);
        }
    }

    pub fn relocate_root(&mut self, root_id: Uuid, path: PathBuf) -> Result<()> {
        let canonical_path = canonical_directory(&path)?;
        let identity = directory_identity(&canonical_path)?;
        let workspace = self
            .active_mut()
            .ok_or_else(|| anyhow!("cannot relocate a root without an active workspace"))?;
        let root_index = workspace
            .roots
            .iter()
            .position(|root| root.id == root_id)
            .ok_or_else(|| anyhow!("workspace root does not exist: {root_id}"))?;

        if workspace
            .roots
            .iter()
            .enumerate()
            .any(|(index, root)| index != root_index && root_matches(root, &identity))
        {
            return Err(anyhow!("workspace already contains that folder"));
        }

        workspace.roots[root_index].path = canonical_path;
        Ok(())
    }

    pub fn remove_root(&mut self, root_id: Uuid) {
        for workspace in &mut self.workspaces {
            let Some(index) = workspace.roots.iter().position(|root| root.id == root_id) else {
                continue;
            };

            workspace.roots.remove(index);
            if workspace.default_root == Some(root_id) {
                workspace.default_root = workspace.roots.first().map(|root| root.id);
            }
            break;
        }
    }

    pub fn rename_workspace(&mut self, name: Option<String>) {
        if let Some(workspace) = self.active_mut() {
            workspace.name = name;
        }
    }

    pub fn remove_workspace(&mut self, workspace_id: Uuid) {
        let Some(index) = self
            .workspaces
            .iter()
            .position(|workspace| workspace.id == workspace_id)
        else {
            return;
        };

        let was_active = self.active_workspace == Some(workspace_id);
        self.workspaces.remove(index);
        if was_active {
            self.active_workspace = self.workspaces.first().map(|workspace| workspace.id);
        }
    }

    pub fn toggle_pin_workspace(&mut self, workspace_id: Uuid) {
        if let Some(workspace) = self
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == workspace_id)
        {
            workspace.pinned = !workspace.pinned;
        }
    }

    pub fn toggle_archive_workspace(&mut self, workspace_id: Uuid) {
        let Some(workspace) = self
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == workspace_id)
        else {
            return;
        };

        workspace.archived = !workspace.archived;
        if workspace.archived && self.active_workspace == Some(workspace_id) {
            self.active_workspace = self
                .workspaces
                .iter()
                .find(|workspace| !workspace.archived)
                .map(|workspace| workspace.id);
        }
    }

    pub fn set_thread_id(&mut self, workspace_id: Uuid, thread_id: String) {
        if let Some(workspace) = self
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == workspace_id)
        {
            workspace.thread_id = Some(thread_id);
        }
    }

    pub fn select_workspace(&mut self, workspace_id: Uuid) {
        if self
            .workspaces
            .iter()
            .any(|workspace| workspace.id == workspace_id)
        {
            self.active_workspace = Some(workspace_id);
        }
    }
}

fn fresh_id(used: &mut HashSet<Uuid>) -> Uuid {
    loop {
        let id = Uuid::new_v4();
        if used.insert(id) {
            return id;
        }
    }
}

fn normalize_window(window: &mut WindowPlacement) -> bool {
    let original = (window.x, window.y, window.width, window.height);
    window.x = finite_clamped(window.x, 0.0, -100_000.0, 100_000.0);
    window.y = finite_clamped(window.y, 0.0, -100_000.0, 100_000.0);
    window.width = finite_clamped(window.width, 1_120.0, 800.0, 10_000.0);
    window.height = finite_clamped(window.height, 760.0, 560.0, 10_000.0);
    original != (window.x, window.y, window.width, window.height)
}

fn finite_clamped(value: f32, fallback: f32, minimum: f32, maximum: f32) -> f32 {
    if value.is_finite() {
        value.clamp(minimum, maximum)
    } else {
        fallback
    }
}
