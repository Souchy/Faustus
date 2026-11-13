use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_updater::UpdaterExt;

#[derive(Default)]
pub struct UpdateCheck(AtomicBool);

pub fn check(app: &AppHandle, manual: bool) {
    let configured = app
        .config()
        .plugins
        .0
        .get("updater")
        .and_then(|config| config.get("pubkey"))
        .and_then(|key| key.as_str())
        .is_some_and(|key| !key.trim().is_empty());
    if !configured {
        if manual {
            app.dialog().message("Updates are not configured in this local build. Install a signed release from GitHub to enable updates.")
                .title("Faust updates").show(|_| {});
        }
        return;
    }
    if app.state::<UpdateCheck>().0.swap(true, Ordering::SeqCst) {
        if manual {
            app.dialog()
                .message("An update check or download is already in progress.")
                .title("Faust updates")
                .show(|_| {});
        }
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = check_and_install(&app, manual).await;
        app.state::<UpdateCheck>().0.store(false, Ordering::SeqCst);
        if let Err(error) = result {
            app.dialog().message(format!("Unable to update Faust:\n\n{error}\n\nYou can retry from Help > Check for updates."))
                .title("Faust updates").kind(MessageDialogKind::Error).show(|_| {});
        }
    });
}

async fn check_and_install(
    app: &AppHandle,
    manual: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let updater = app
        .updater_builder()
        .timeout(Duration::from_secs(120))
        .build()?;
    let Some(update) = updater.check().await? else {
        if manual {
            app.dialog()
                .message(format!(
                    "Faust {} is up to date.",
                    app.package_info().version
                ))
                .title("Faust updates")
                .show(|_| {});
        }
        return Ok(());
    };
    let (answer, response) = tokio::sync::oneshot::channel();
    app.dialog().message(format!(
        "Faust {} is available (installed: {}).\n\nDownload and install it now? The app will close and restart. Saved league data will be kept.",
        update.version, app.package_info().version
    )).title("Faust update available")
        .buttons(MessageDialogButtons::OkCancelCustom("Update now".into(), "Later".into()))
        .show(move |accepted| { let _ = answer.send(accepted); });
    if !response.await? {
        return Ok(());
    }
    // Verify the signature before allowing the installer to close the running app.
    let bytes = update.download(|_, _| {}, || {}).await?;
    update.install(bytes)?;
    Ok(())
}
