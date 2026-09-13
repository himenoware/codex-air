//! A single writer owns workspace state. Disk work never runs on the UI thread.
use crate::{
    storage,
    workspace::{AppState, WindowPlacement},
};
use std::{path::PathBuf, sync::mpsc, thread};
use uuid::Uuid;

#[derive(Clone)]
pub enum Command {
    Open(PathBuf),
    AddRoot(Uuid, PathBuf),
    RelocateRoot(Uuid, Uuid, PathBuf),
    RemoveRoot(Uuid, Uuid),
    DefaultRoot(Uuid, Uuid),
    Select(Uuid),
    Rename(Uuid, String),
    RemoveWorkspace(Uuid),
    TogglePin(Uuid),
    ToggleArchive(Uuid),
    SetThread(Uuid, String),
    Refresh,
    Placement(WindowPlacement),
    Sidebar(f32),
    Preferences(crate::workspace::Preferences),
    Close,
}

pub struct Snapshot {
    pub state: AppState,
    pub warning: Option<String>,
    pub initial: bool,
    pub closed: bool,
    pub close_failed: bool,
    pub can_save: bool,
}

pub fn start() -> (mpsc::Sender<Command>, async_channel::Receiver<Snapshot>) {
    let (send, recv) = mpsc::channel();
    let (updates, receiver) = async_channel::unbounded();
    thread::Builder::new().name("air-workspaces".into()).spawn(move || {
        let path = storage::default_state_path();
        let loaded = storage::load(&path);
        let can_save = loaded.can_save;
        let mut state = loaded.state;
        let mut warning = loaded.warning;
        let save = |state: &AppState| -> anyhow::Result<()> {
            anyhow::ensure!(can_save, "Saving is disabled to protect the original state file. Resolve its read/backup error and restart Codex Air.");
            storage::save(&path, state)
        };
        let snapshot = |state: &AppState, warning: Option<String>, initial, closed| Snapshot {
            state: state.clone(),
            warning, initial, closed, close_failed: false, can_save,
        };
        if updates.send_blocking(snapshot(&state, warning.clone(), true, false)).is_err() { return; }
        let mut dirty = false;
        loop {
            let received = if dirty { recv.recv_timeout(std::time::Duration::from_millis(300)) }
                else { recv.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected) };
            let command = match received {
                Ok(command) => command,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if dirty {
                        dirty = false;
                        if let Err(error) = save(&state) {
                            warning = Some(format!("Could not save workspace state: {error:#}"));
                            let _ = updates.send_blocking(snapshot(&state, warning.clone(), false, false));
                        }
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => { let _ = save(&state); break; }
            };
            match command {
                Command::Placement(placement) => { state.window = Some(placement); dirty = can_save; continue; }
                Command::Sidebar(width) => { state.sidebar_width = width.clamp(200., 380.); dirty = can_save; continue; }
                Command::Close => {
                    if !can_save { let _ = updates.send_blocking(snapshot(&state, None, false, true)); break; }
                    match save(&state) {
                        Ok(()) => { let _ = updates.send_blocking(snapshot(&state, None, false, true)); break; }
                        Err(error) => {
                            warning = Some(format!("Could not save before closing: {error:#}"));
                            let mut failed = snapshot(&state, warning.clone(), false, false);
                            failed.close_failed = true;
                            let _ = updates.send_blocking(failed);
                            continue;
                        }
                    }
                }
                _ => {}
            }
            let result: anyhow::Result<()> = (|| {
                anyhow::ensure!(can_save || matches!(command, Command::Select(_) | Command::Refresh),
                    "Workspace changes are disabled to protect your saved state. Resolve the state file error and restart Codex Air.");
                match command {
                    Command::Open(path) => { state.open_folder(path)?; }
                    Command::AddRoot(workspace, path) => with_workspace(&mut state, workspace, |state| { state.add_root(path)?; Ok(()) })?,
                    Command::RelocateRoot(workspace, root, path) => with_workspace(&mut state, workspace, |state| state.relocate_root(root, path))?,
                    Command::RemoveRoot(workspace, root) => with_workspace(&mut state, workspace, |state| { state.remove_root(root); Ok(()) })?,
                    Command::DefaultRoot(workspace, root) => with_workspace(&mut state, workspace, |state| { state.set_default_root(root); Ok(()) })?,
                    Command::Select(id) => { state.select_workspace(id); }
                    Command::Rename(id, name) => with_workspace(&mut state, id, |state| { state.rename_workspace(Some(name.trim().to_owned())); Ok(()) })?,
                    Command::RemoveWorkspace(id) => { state.remove_workspace(id); }
                    Command::TogglePin(id) => { state.toggle_pin_workspace(id); }
                    Command::ToggleArchive(id) => { state.toggle_archive_workspace(id); }
                    Command::SetThread(id, thread_id) => { state.set_thread_id(id, thread_id); }
                    Command::Refresh => {}
                    Command::Preferences(preferences) => { state.preferences = preferences; }
                    _ => unreachable!(),
                }
                Ok(())
            })();
            match result {
                Ok(()) => {
                    state.normalize();
                    if can_save && let Err(error) = save(&state) {
                        warning = Some(format!("Changes are in memory but could not be saved: {error:#}"));
                    }
                }
                Err(error) => warning = Some(format!("{error:#}")),
            }
            dirty = false;
            if updates.send_blocking(snapshot(&state, warning.clone(), false, false)).is_err() { break; }
            warning = None;
        }
    }).expect("Could not start workspace storage worker");
    (send, receiver)
}

fn with_workspace(
    state: &mut AppState,
    id: Uuid,
    change: impl FnOnce(&mut AppState) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        state.workspaces.iter().any(|w| w.id == id),
        "This workspace was removed. Open the folder again."
    );
    let previous = state.active_workspace;
    state.active_workspace = Some(id);
    let result = change(state);
    state.active_workspace = previous;
    result
}
