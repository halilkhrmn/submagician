// No console window behind the app in Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod controller;

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let settings = submagician_core::settings::Settings::load();
    let ui = AppWindow::new()?;

    let runtime =
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("tokio runtime");
    controller::Controller::start(&ui, settings, runtime.handle().clone());

    ui.run()?;
    runtime.shutdown_background();
    Ok(())
}
