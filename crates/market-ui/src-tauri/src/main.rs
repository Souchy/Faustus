#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(any(not(debug_assertions), test))]
mod backend;
mod updates;

fn main() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(updates::UpdateCheck::default())
        .on_menu_event(|app, event| {
            if event.id() == "check-updates" {
                updates::check(app, true);
            }
        });
    let builder = builder.setup(|app| {
        #[cfg(not(debug_assertions))]
        {
            use tauri::Manager;
            use tauri_plugin_dialog::DialogExt;

            let result = (|| {
                let directory = app.path().app_data_dir()?;
                std::fs::create_dir_all(&directory)?;
                tauri::async_runtime::block_on(backend::Backend::start(
                    &directory.join("faust.sqlite3"),
                    "127.0.0.1:8787".parse()?,
                ))
            })();
            let backend = match result {
                Ok(backend) => Some(backend),
                Err(error) => {
                    let handle = app.handle().clone();
                    app.dialog()
                        .message(format!("Faust could not start:\n\n{error}"))
                        .title("Faust")
                        .kind(tauri_plugin_dialog::MessageDialogKind::Error)
                        .show(move |_| handle.exit(1));
                    app.manage(std::sync::Mutex::new(None::<backend::Backend>));
                    return Ok(());
                }
            };
            app.manage(std::sync::Mutex::new(backend));
        }
        let help = tauri::menu::SubmenuBuilder::new(app, "Help")
            .text("check-updates", "Check for updates...")
            .build()?;
        app.set_menu(tauri::menu::MenuBuilder::new(app).item(&help).build()?)?;
        tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?.build()?;
        #[cfg(not(debug_assertions))]
        updates::check(app.handle(), false);
        Ok(())
    });
    let app = builder
        .build(tauri::generate_context!())
        .expect("failed to build Faust desktop");
    app.run(|_app, _event| {
        #[cfg(not(debug_assertions))]
        if let tauri::RunEvent::ExitRequested { api, code, .. } = _event {
            use tauri::Manager;
            use tauri_plugin_dialog::DialogExt;

            let state = _app.state::<std::sync::Mutex<Option<backend::Backend>>>();
            if let Some(backend) = state.lock().expect("backend state poisoned").take() {
                api.prevent_exit();
                let handle = _app.clone();
                tauri::async_runtime::spawn(async move {
                    match backend.stop().await {
                        Ok(()) => handle.exit(code.unwrap_or(0)),
                        Err(error) => {
                            let exit_handle = handle.clone();
                            handle
                                .dialog()
                                .message(format!("Faust service shutdown failed:\n\n{error}"))
                                .title("Faust")
                                .kind(tauri_plugin_dialog::MessageDialogKind::Error)
                                .show(move |_| exit_handle.exit(1));
                        }
                    }
                });
            }
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn installer_and_updater_use_the_frontend_package_version() {
        let package: serde_json::Value =
            serde_json::from_str(include_str!("../../package.json")).unwrap();
        let context: tauri::Context<tauri::Wry> = tauri::generate_context!();
        assert_eq!(context.config().product_name.as_deref(), Some("Faust"));
        assert_eq!(context.config().identifier, "com.faust.market");
        assert_eq!(context.config().app.windows[0].title, "Faust");
        assert_eq!(
            context.package_info().version.to_string(),
            package["version"].as_str().unwrap()
        );
    }
}
