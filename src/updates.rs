//! Explicit, background-only release discovery using Windows' HTTP stack.
use anyhow::{Result, bail};
use gpui_kit::AppContext;
use std::{ffi::c_void, ptr};
use windows::{
    Win32::Networking::WinHttp::*,
    core::{PCWSTR, w},
};

struct HttpHandle(*mut c_void);
impl HttpHandle {
    fn checked(raw: *mut c_void) -> Result<Self> {
        if raw.is_null() {
            return Err(windows::core::Error::from_win32().into());
        }
        Ok(Self(raw))
    }
}
impl Drop for HttpHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}

pub struct LatestRelease {
    pub version: String,
    pub newer: bool,
}

pub struct UpdateView {
    message: String,
}

impl UpdateView {
    pub fn new(cx: &mut gpui_kit::Context<Self>) -> Self {
        let job = cx.background_spawn(async { check() });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.message = match result {
                    Ok(release) if release.newer => {
                        format!("Codex Air {} is available.", release.version)
                    }
                    Ok(release) => format!(
                        "Installed: {}. Latest published release: {}.",
                        env!("CARGO_PKG_VERSION"),
                        release.version
                    ),
                    Err(error) => format!("Could not check for updates: {error}"),
                };
                cx.notify();
            });
        })
        .detach();
        Self {
            message: "Checking GitHub for the latest release…".into(),
        }
    }
}
impl gpui_kit::Render for UpdateView {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        use gpui_kit::ParentElement;
        gpui_kit::div().child(self.message.clone())
    }
}

pub fn check() -> Result<LatestRelease> {
    // All handles belong to this worker call. No UI thread or credentials involved.
    let body = unsafe {
        let session = HttpHandle::checked(WinHttpOpen(
            w!("Codex-Air-update-check"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ))?;
        WinHttpSetTimeouts(session.0, 8000, 8000, 8000, 8000)?;
        let connection =
            HttpHandle::checked(WinHttpConnect(session.0, w!("api.github.com"), 443, 0))?;
        let request = HttpHandle::checked(WinHttpOpenRequest(
            connection.0,
            w!("GET"),
            w!("/repos/himenoware/codex-air/releases/latest"),
            PCWSTR::null(),
            PCWSTR::null(),
            ptr::null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        WinHttpSendRequest(request.0, None, None, 0, 0, 0)?;
        WinHttpReceiveResponse(request.0, ptr::null_mut())?;
        let mut status = 0u32;
        let mut status_size = 4u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status as *mut u32).cast()),
            &mut status_size,
            ptr::null_mut(),
        )?;
        if status != 200 {
            bail!("GitHub returned HTTP {status}. Try again later.");
        }
        let mut body = Vec::new();
        loop {
            let mut buffer = [0u8; 8192];
            let mut count = 0;
            WinHttpReadData(
                request.0,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut count,
            )?;
            if count == 0 {
                break;
            }
            body.extend_from_slice(&buffer[..count as usize]);
            if body.len() > 1_048_576 {
                bail!("Release response exceeded the expected size.");
            }
        }
        body
    };
    let release: serde_json::Value = serde_json::from_slice(&body)?;
    let version = release
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("GitHub did not return a release version."))?
        .trim_start_matches('v')
        .to_owned();
    let parse = |version: &str| -> Result<(u64, u64, u64)> {
        let parts: Vec<_> = version.split('.').collect();
        if parts.len() != 3 {
            bail!("Unsupported release version: {version}");
        }
        Ok((parts[0].parse()?, parts[1].parse()?, parts[2].parse()?))
    };
    let newer = parse(&version)? > parse(env!("CARGO_PKG_VERSION"))?;
    Ok(LatestRelease { version, newer })
}
