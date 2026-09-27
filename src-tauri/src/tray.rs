use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::jwt::JwtStatusView;

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "打开配置", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 cannbot-adapter", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    TrayIconBuilder::with_id("main")
        .icon(
            app.default_window_icon()
                .expect("app icon missing")
                .clone(),
        )
        .tooltip("CANNBot Adapter · 启动中")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

pub fn set_status_tooltip(app: &AppHandle, jwt: &JwtStatusView) {
    if let Some(tray) = app.tray_by_id("main") {
        let status = if jwt.ok { "运行中" } else { "JWT 未就绪" };
        let _ = tray.set_tooltip(Some(&format!("CANNBot Adapter · {status}")));
    }
}
