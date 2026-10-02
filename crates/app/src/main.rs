// No console window behind the app in Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod controller;

slint::include_modules!();

/// Folders and videos dropped on the window open like "Choose folder…". winit reports drops on
/// Windows, X11 and macOS; on Wayland it has no drop support yet.
fn accept_dropped_files(ui: &AppWindow, controller: controller::Controller) {
    use slint::winit_030::{EventResult, WinitWindowAccessor, winit};
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

fn main() -> Result<(), slint::PlatformError> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let settings = submagician_core::settings::Settings::load();
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
