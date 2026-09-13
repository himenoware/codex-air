#![windows_subsystem = "windows"]

use gpui_kit::{
    component::{Root, TitleBar},
    *,
};
mod app_server;
mod controller;
mod diagnostics;
mod platform;
mod storage;
mod theme;
mod ui;
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
    let window_instance = instance.clone();
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);
    diagnostics::mark("platform_created");
    app.run(move |cx| {
        diagnostics::mark("event_loop_ready");
        gpui_kit::init(cx);
        diagnostics::mark("components_initialized");
        theme::init(cx);
        diagnostics::mark("theme_ready");
        ui::bind_keys(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1120.), px(760.)),
                cx,
            ))),
            window_min_size: Some(size(px(800.), px(560.))),
            app_id: Some("CodexAir.Desktop".into()),
            ..TitleBar::window_options()
        };
        cx.open_window(options, |window, cx| {
            window_instance
                .register(window)
                .expect("Could not register the desktop window");
            let view = cx.new(|cx| ui::Shell::new(window, cx));
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
