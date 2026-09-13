//! Windows desktop integration.
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use gpui_kit::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, RECT},
    Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect},
    System::Threading::CreateMutexW,
    UI::WindowsAndMessaging::{
        EnumWindows, GetPropW, GetWindowPlacement, IsIconic, MessageBoxW, SW_RESTORE,
        SW_SHOWMAXIMIZED, SW_SHOWNORMAL, SetForegroundWindow, SetPropW, SetWindowPlacement,
        ShowWindow, WINDOWPLACEMENT, WPF_RESTORETOMAXIMIZED,
    },
};
use windows::core::BOOL;

fn hwnd(window: &Window) -> Option<HWND> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut _)),
        _ => None,
    }
}

pub struct InstanceGuard {
    mutex: HANDLE,
    property_name: Vec<u16>,
}

impl InstanceGuard {
    pub fn acquire() -> Result<Option<Self>> {
        let data_directory = normalized_data_directory()?;
        let key = stable_path_key(&data_directory);
        let mutex_name = wide_null(&format!(r"Local\CodexAir.Instance.{key:016x}"));
        let property_name = wide_null(&format!("CodexAir.Window.{key:016x}"));
        let mutex =
            unsafe { CreateMutexW(None, false, windows::core::PCWSTR(mutex_name.as_ptr())) }
                .map_err(|error| anyhow!("create Codex Air instance mutex: {error}"))?;

        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            let existing = match find_registered_window(&property_name) {
                Ok(existing) => existing,
                Err(error) => {
                    unsafe {
                        let _ = CloseHandle(mutex);
                    }
                    return Err(error);
                }
            };
            if let Some(window) = existing {
                unsafe {
                    // Restoring an already maximized window would silently turn it into a
                    // normal window. Only restore a minimized instance; Windows preserves a
                    // maximized restore state when SW_RESTORE is applied to that case.
                    if IsIconic(window).as_bool() {
                        let _ = ShowWindow(window, SW_RESTORE);
                    }
                    let _ = SetForegroundWindow(window);
                }
            } else {
                show_starting_message();
            }
            unsafe {
                let _ = CloseHandle(mutex);
            }
            return Ok(None);
        }

        Ok(Some(Self {
            mutex,
            property_name,
        }))
    }

    pub fn register(&self, window: &Window) -> Result<()> {
        let handle = hwnd(window).ok_or_else(|| anyhow!("Codex Air window has no Win32 handle"))?;
        unsafe {
            SetPropW(
                handle,
                windows::core::PCWSTR(self.property_name.as_ptr()),
                Some(HANDLE(std::ptr::dangling_mut::<std::ffi::c_void>())),
            )
            .map_err(|error| anyhow!("register Codex Air window: {error}"))
        }
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.mutex);
        }
    }
}

struct WindowSearch<'a> {
    property_name: &'a [u16],
    found: Option<HWND>,
}

unsafe extern "system" fn find_window_callback(window: HWND, data: LPARAM) -> BOOL {
    let search = unsafe { &mut *(data.0 as *mut WindowSearch<'_>) };
    if !unsafe { GetPropW(window, windows::core::PCWSTR(search.property_name.as_ptr())) }
        .is_invalid()
    {
        search.found = Some(window);
        return BOOL(0);
    }
    BOOL(1)
}

fn find_registered_window(property_name: &[u16]) -> Result<Option<HWND>> {
    let mut search = WindowSearch {
        property_name,
        found: None,
    };
    let result = unsafe {
        EnumWindows(
            Some(find_window_callback),
            LPARAM(&mut search as *mut WindowSearch<'_> as isize),
        )
    };
    // Stopping enumeration after a match returns FALSE too; GetLastError can
    // still contain ERROR_ALREADY_EXISTS from CreateMutexW.
    if search.found.is_none() {
        result.map_err(|error| anyhow!("find existing Codex Air window: {error}"))?;
    }
    Ok(search.found)
}

fn show_starting_message() {
    let message = wide_null("Codex Air is already starting");
    let title = wide_null("Codex Air");
    unsafe {
        let _ = MessageBoxW(
            None,
            windows::core::PCWSTR(message.as_ptr()),
            windows::core::PCWSTR(title.as_ptr()),
            Default::default(),
        );
    }
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn normalized_data_directory() -> Result<PathBuf> {
    let directory = crate::storage::default_state_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let absolute = if directory.is_absolute() {
        directory
    } else {
        std::env::current_dir()
            .context("resolve current directory for instance identity")?
            .join(directory)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(
                    normalized.components().next_back(),
                    Some(Component::Normal(_))
                ) {
                    let _ = normalized.pop();
                }
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    Ok(normalized)
}

fn stable_path_key(path: &Path) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in path.to_string_lossy().to_ascii_lowercase().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub fn saved_display(saved: &crate::workspace::WindowPlacement) -> Option<gpui_kit::DisplayId> {
    let rect = RECT {
        left: saved.x as i32,
        top: saved.y as i32,
        right: (saved.x + saved.width) as i32,
        bottom: (saved.y + saved.height) as i32,
    };
    let monitor = unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST) };
    (!monitor.is_invalid()).then(|| gpui_kit::DisplayId::new(monitor.0 as u64))
}

pub fn placement(window: &Window) -> Option<crate::workspace::WindowPlacement> {
    let handle = hwnd(window)?;
    let mut native = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    // SAFETY: GPUI owns a live HWND and native points to a correctly sized structure.
    unsafe {
        GetWindowPlacement(handle, &mut native).ok()?;
    }

    let mut rect = native.rcNormalPosition;
    let mut monitor = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // rcNormalPosition is in workspace coordinates. Convert it to screen coordinates
    // before handing it to the cross-platform state layer.
    unsafe {
        let monitor_handle =
            windows::Win32::Graphics::Gdi::MonitorFromWindow(handle, MONITOR_DEFAULTTONEAREST);
        if GetMonitorInfoW(monitor_handle, &mut monitor).as_bool() {
            let offset_x = monitor.rcWork.left - monitor.rcMonitor.left;
            let offset_y = monitor.rcWork.top - monitor.rcMonitor.top;
            rect.left += offset_x;
            rect.right += offset_x;
            rect.top += offset_y;
            rect.bottom += offset_y;
        }
    }

    let maximized = native.showCmd == SW_SHOWMAXIMIZED.0 as u32
        || (native.flags & WPF_RESTORETOMAXIMIZED).0 != 0;
    Some(crate::workspace::WindowPlacement {
        x: rect.left as f32,
        y: rect.top as f32,
        width: (rect.right - rect.left) as f32,
        height: (rect.bottom - rect.top) as f32,
        maximized,
    })
}

pub fn restore(window: &Window, saved: &crate::workspace::WindowPlacement) {
    let Some(handle) = hwnd(window) else {
        return;
    };
    let width = saved.width.clamp(800., 7680.) as i32;
    let height = saved.height.clamp(560., 4320.) as i32;
    let mut rect = RECT {
        left: saved.x as i32,
        top: saved.y as i32,
        right: saved.x as i32 + width,
        bottom: saved.y as i32 + height,
    };
    let mut monitor = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: all native structures are initialized and live for the call.
    unsafe {
        let monitor_handle = MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST);
        if GetMonitorInfoW(monitor_handle, &mut monitor).as_bool() {
            let work = monitor.rcWork;
            let width = width.min(work.right - work.left);
            let height = height.min(work.bottom - work.top);
            rect.left = rect.left.clamp(work.left, work.right - width);
            rect.top = rect.top.clamp(work.top, work.bottom - height);
            rect.right = rect.left + width;
            rect.bottom = rect.top + height;

            // SetWindowPlacement expects workspace coordinates for normal windows.
            let offset_x = work.left - monitor.rcMonitor.left;
            let offset_y = work.top - monitor.rcMonitor.top;
            rect.left -= offset_x;
            rect.right -= offset_x;
            rect.top -= offset_y;
            rect.bottom -= offset_y;
        }
        let mut native = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        if GetWindowPlacement(handle, &mut native).is_ok() {
            native.rcNormalPosition = rect;
            native.showCmd = if saved.maximized {
                SW_SHOWMAXIMIZED.0 as u32
            } else {
                SW_SHOWNORMAL.0 as u32
            };
            native.flags &= !WPF_RESTORETOMAXIMIZED;
            let _ = SetWindowPlacement(handle, &native);
            // GPUI creates its initial window before the asynchronous state
            // load completes. Explicitly applying the saved show state avoids
            // leaving a maximized placement as a merely normal-sized window.
            let _ = ShowWindow(
                handle,
                if saved.maximized {
                    SW_SHOWMAXIMIZED
                } else {
                    SW_SHOWNORMAL
                },
            );
        }
    }
}
