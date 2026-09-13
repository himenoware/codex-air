#![windows_subsystem = "windows"]

use gpui_kit::{
    component::{Root, TitleBar},
    *,
};
mod app_server;
mod assets;
mod codex_settings;
mod composer_controls;
mod controller;
mod diagnostics;
mod platform;
mod preferences;
mod questions;
mod releases;
mod storage;
mod theme;
mod ui;
mod updates;
mod workspace;

fn main() {
    std::panic::set_hook(Box::new(|info| {
        use windows::{
            Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MessageBoxW},
            core::{HSTRING, w},
        };
        let message = HSTRING::from(format!("Codex Air could not continue.\n\n{info}"));
        unsafe {
            MessageBoxW(None, &message, w!("Codex Air"), MB_ICONERROR);
        }
    }));
    diagnostics::init();
    let Some(instance) =
        platform::InstanceGuard::acquire().expect("Could not initialize the desktop instance")
    else {
        return;
    };
    let instance = std::rc::Rc::new(instance);
    let saved_window = storage::load(&storage::default_state_path()).state.window;
    let window_instance = instance.clone();
    let app = gpui_kit::application().with_assets(assets::Assets);
    diagnostics::mark("platform_created");
    app.run(move |cx| {
        diagnostics::mark("event_loop_ready");
        gpui_kit::init(cx);
        diagnostics::mark("components_initialized");
        theme::init(cx);
        diagnostics::mark("theme_ready");
        ui::bind_keys(cx);
        let initial_bounds = match saved_window.as_ref() {
            Some(saved) if saved.maximized => WindowBounds::Maximized(
                cx.displays()
                    .into_iter()
                    .find(|display| Some(display.id()) == platform::saved_display(saved))
                    .or_else(|| cx.primary_display())
                    .map(|display| display.visible_bounds())
                    .unwrap_or_else(|| {
                        Bounds::centered(None, size(px(saved.width), px(saved.height)), cx)
                    }),
            ),
            Some(saved) => WindowBounds::Windowed(Bounds::new(
                point(px(saved.x), px(saved.y)),
                size(px(saved.width), px(saved.height)),
            )),
            None => WindowBounds::Windowed(Bounds::centered(None, size(px(1120.), px(760.)), cx)),
        };
        // GPUI applies a maximized WindowBounds asynchronously after creating the HWND. Keep a
        // restored maximized window hidden until the native placement is applied so Windows never
        // paints the saved normal-size rectangle for a frame during startup.
        let startup_maximized = saved_window.as_ref().is_some_and(|saved| saved.maximized);
        let options = WindowOptions {
            window_bounds: Some(initial_bounds),
            window_min_size: Some(size(px(800.), px(560.))),
            show: !startup_maximized,
            display_id: saved_window.as_ref().and_then(platform::saved_display),
            app_id: Some("CodexAir.Desktop".into()),
            ..TitleBar::window_options()
        };
        let startup_window = saved_window.clone();
        cx.open_window(options, |window, cx| {
            window_instance
                .register(window)
                .expect("Could not register the desktop window");
            let view = cx.new(|cx| ui::Shell::new(window, cx));
            if startup_maximized && let Some(saved) = startup_window {
                // Activate the specific GPUI window to consume its hidden initial placement.
                // A hidden window does not produce frames. Its initial client bounds already
                // fill the monitor work area; only restore metadata remains for the first frame.
                window.activate_window();
                window.on_next_frame(move |window, _| platform::restore(window, &saved));
            }
            cx.new(|cx| Root::new(view, window, cx))
        })
        .expect("Could not open Codex Air");
        diagnostics::mark("window_created");
        cx.activate(true);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
    });
    drop(instance);
}
