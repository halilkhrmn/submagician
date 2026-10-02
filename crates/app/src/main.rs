// No console window behind the app in Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod controller;
#[cfg(target_os = "linux")]
mod wayland_drop;

slint::include_modules!();

/// Folders and videos dropped on the window open like "Choose folder…". winit reports drops on
/// Windows, X11 and macOS; Wayland is handled by `wayland_drop`.
fn accept_dropped_files(ui: &AppWindow, controller: controller::Controller) {
    use slint::winit_030::{EventResult, WinitWindowAccessor, winit};
    #[cfg(target_os = "linux")]
    accept_wayland_drops(ui, controller.clone());
    let last: std::cell::RefCell<Option<std::time::Instant>> = Default::default();
    ui.window().on_winit_window_event(move |_, event| {
        if let winit::event::WindowEvent::DroppedFile(path) = event {
            // Several files dropped at once arrive one by one: open the first only.
            let now = std::time::Instant::now();
            let recent = last.borrow().is_some_and(|t| now.duration_since(t).as_millis() < 1000);
            *last.borrow_mut() = Some(now);
            if !recent {
                let (c, path) = (controller.clone(), path.clone());
                slint::Timer::single_shot(std::time::Duration::ZERO, move || c.open_path(path));
            }
        }
        EventResult::Propagate
    });
}

/// Under Wayland, listen for drops on winit's connection ourselves (see `wayland_drop`). The
/// winit window exists only once the event loop runs, hence the timer.
#[cfg(target_os = "linux")]
fn accept_wayland_drops(ui: &AppWindow, controller: controller::Controller) {
    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
    let weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(300), move || {
        let Some(ui) = weak.upgrade() else { return };
        let display = ui.window().with_winit_window(|w| match w.display_handle().map(|h| h.as_raw()) {
            Ok(RawDisplayHandle::Wayland(d)) => Some(d.display.as_ptr()),
            _ => None,
        });
        let Some(Some(display)) = display else { return };
        let started = wayland_drop::start(display, move |path| {
            let c = controller.clone();
            let _ = slint::invoke_from_event_loop(move || c.open_path(path));
        });
        match started {
            Ok(()) => log::info!("Wayland drops enabled"),
            Err(e) => log::warn!("Wayland drops not available: {e}"),
        }
    });
}

fn main() -> Result<(), slint::PlatformError> {
    let settings = submagician_core::settings::Settings::load();
    // Problems always go to errors.log; every line to daily files when the user asked for that.
    submagician_core::applog::init(settings.detailed_logs);
    log::info!("SubMagician {} starting on {}", submagician_core::VERSION, submagician_core::report::os_description());
    let ui = AppWindow::new()?;

    let runtime =
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("tokio runtime");
    let initial = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let controller = controller::Controller::start(&ui, settings, runtime.handle().clone(), initial);
    accept_dropped_files(&ui, controller);

    ui.run()?;
    runtime.shutdown_background();
    Ok(())
}
