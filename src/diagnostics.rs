//! Opt-in local startup measurements; no network or workspace contents.
use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::Instant,
};

struct Trace {
    start: Instant,
    path: PathBuf,
    phases: Mutex<Vec<(&'static str, f64)>>,
}
static TRACE: OnceLock<Option<Trace>> = OnceLock::new();

pub fn init() {
    TRACE.get_or_init(|| {
        std::env::var_os("CODEX_AIR_STARTUP_LOG").map(|path| Trace {
            start: Instant::now(),
            path: path.into(),
            phases: Mutex::new(Vec::new()),
        })
    });
}

pub fn mark(phase: &'static str) {
    if let Some(Some(trace)) = TRACE.get() {
        trace
            .phases
            .lock()
            .unwrap()
            .push((phase, trace.start.elapsed().as_secs_f64() * 1000.));
    }
}

pub fn flush() {
    use std::io::Write;
    if let Some(Some(trace)) = TRACE.get() {
        let phases: std::collections::BTreeMap<_, _> =
            trace.phases.lock().unwrap().iter().copied().collect();
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&trace.path)
        {
            let _ = writeln!(
                file,
                "{}",
                serde_json::json!({"pid":std::process::id(), "phases_ms":phases})
            );
        }
    }
}
