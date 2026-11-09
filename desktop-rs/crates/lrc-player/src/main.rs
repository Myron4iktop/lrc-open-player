// No console window behind the player in release builds, like PyInstaller's `--windowed`.
// Debug builds keep the console so `println!` and panics are visible while developing.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

slint::include_modules!();

mod app;
mod audio;
mod platform;
mod session;

fn main() -> Result<(), slint::PlatformError> {
    #[cfg(windows)]
    platform::set_app_user_model_id();

    // Slint's software renderer draws the small transport icons crisply, as the Python
    // app's QPainter does; its OpenGL renderer blurs shapes this small. View → Smooth
    // animation picks the OpenGL one instead, which keeps up with 120 Hz screens; if the
    // GPU can't be used, the software one is used after all. `SLINT_BACKEND` still picks
    // another one, for comparing.
    if std::env::var_os("SLINT_BACKEND").is_none() {
        let gpu = session::smooth_animation_wanted()
            && slint::BackendSelector::new()
                .backend_name("winit".into())
                .renderer_name("femtovg".into())
                .select()
                .is_ok();
        if !gpu {
            slint::BackendSelector::new()
                .backend_name("winit".into())
                .renderer_name("software".into())
                .select()?;
        }
    }

    let window = MainWindow::new()?;
    // Theme, volume, offset, shuffle, repeat and the window's place, before it is shown.
    let (app, _timers) = app::attach(&window);
    window.show()?;
    // The native window exists only once shown.
    #[cfg(windows)]
    platform::set_icons_from_exe(&window);
    platform::apply_title_bar(&window, window.global::<Theme>().get_dark());
    platform::hook_window_events(&window, &app);
    platform::keep_on_screen(&window);
    window.invoke_focus_window();
    // The last playlist comes back once the window is up (`QTimer.singleShot(0, …)`),
    // unless a debug build was told to open something else.
    let dev_open = cfg!(debug_assertions) && std::env::var_os("LRC_DEV_OPEN").is_some();
    let weak = std::rc::Rc::downgrade(&app);
    let weak_ui = window.as_weak();
    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
        if !dev_open
            && let (Some(app), Some(ui)) = (weak.upgrade(), weak_ui.upgrade())
            && let Ok(mut app) = app.try_borrow_mut()
        {
            app.restore_playback_session(&ui);
        }
    });
    // Developer aid in debug builds only: `LRC_DEV_OPEN=<folder or song>` opens it at
    // start, for checking the window without a file dialog.
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("LRC_DEV_OPEN") {
        app.borrow_mut().open_path(&window, path.into());
    }
    slint::run_event_loop()?;
    window.hide()
}
