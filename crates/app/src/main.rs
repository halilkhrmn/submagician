// No console window behind the app in Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod controller;
mod settings;

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let settings = settings::Settings::load();
    let ui = AppWindow::new()?;
    if let Err(e) = slint::select_bundled_translation(settings.ui_translation()) {
        log::warn!("translation not selected: {e}");
    }

    let runtime =
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("tokio runtime");
    controller::Controller::start(&ui, settings, runtime.handle().clone());

    ui.run()?;
    runtime.shutdown_background();
    Ok(())
}
