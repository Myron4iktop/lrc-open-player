// No console window behind the player in release builds, like PyInstaller's `--windowed`.
// Debug builds keep the console so `println!` and panics are visible while developing.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

slint::include_modules!();

mod app;
mod audio;
mod platform;

fn main() -> Result<(), slint::PlatformError> {
    #[cfg(windows)]
    platform::set_app_user_model_id();

    // Slint's software renderer draws the small transport icons crisply, as the Python
    // app's QPainter does; its OpenGL renderer blurs shapes this small. `SLINT_BACKEND`
    // still picks another one, for comparing.
    if std::env::var_os("SLINT_BACKEND").is_none() {
        slint::BackendSelector::new()
            .backend_name("winit".into())
            .renderer_name("software".into())
            .select()?;
    }

    let window = MainWindow::new()?;
    let (app, _ticker) = app::attach(&window);
    window.show()?;
    // The native window exists only once shown.
    #[cfg(windows)]
    platform::set_icons_from_exe(&window);
    platform::apply_title_bar(&window, window.global::<Theme>().get_dark());
    platform::hook_file_drops(&window, &app);
    window.invoke_focus_window();
    // Developer aid in debug builds only: `LRC_DEV_OPEN=<folder or song>` opens it at
    // start, for checking the window without a file dialog.
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("LRC_DEV_OPEN") {
        app.borrow_mut().open_path(&window, path.into());
    }
    slint::run_event_loop()?;
    window.hide()
}
