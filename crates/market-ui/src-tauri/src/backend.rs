use std::{io, net::SocketAddr, path::Path, time::Duration};

use market_service::{ServiceState, cors_layer, economy, router, run_periodic_gem_collector};
use market_store::SqliteStore;
use tokio::{sync::oneshot, task::JoinHandle};

pub struct Backend {
    shutdown: Option<oneshot::Sender<()>>,
    server: JoinHandle<io::Result<()>>,
    collectors: [JoinHandle<()>; 2],
    #[cfg(test)]
    address: SocketAddr,
}

impl Backend {
    pub async fn start(
        database: &Path,
        address: SocketAddr,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Bind before opening storage so a second instance cannot recover an active import.
        let listener = tokio::net::TcpListener::bind(address).await.map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("Cannot start Faust on {address}. Close any other Faust app or market service and try again. {error}"),
            )
        })?;
        #[cfg(test)]
        let address = listener.local_addr()?;
        let store = SqliteStore::connect(database).await?;
        let (selection, selected) = tokio::sync::watch::channel(None);
        let state = ServiceState::new(store, "Faust/0.1.0", selection);
        economy::recover_imports(&state).await?;
        let origins = [
            "tauri://localhost",
            "http://tauri.localhost",
            "https://tauri.localhost",
        ]
        .map(axum::http::HeaderValue::from_static);
        let app = router(state.clone()).layer(cors_layer(origins.to_vec()));
        let (shutdown, stopped) = oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
        });
        let collectors = [
            tokio::spawn(run_periodic_gem_collector(state.clone(), selected.clone())),
            tokio::spawn(economy::collect(state, selected)),
        ];
        Ok(Self {
            shutdown: Some(shutdown),
            server,
            collectors,
            #[cfg(test)]
            address,
        })
    }

    pub async fn stop(mut self) -> io::Result<()> {
        for collector in &self.collectors {
            collector.abort();
        }
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        match tokio::time::timeout(Duration::from_secs(5), &mut self.server).await {
            Ok(result) => result.map_err(io::Error::other)?,
            Err(error) => Err(io::Error::new(io::ErrorKind::TimedOut, error)),
        }
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        for collector in &self.collectors {
            collector.abort();
        }
        self.server.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fresh_install_starts_empty_and_keeps_its_database_across_restarts() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("faust.sqlite3");
        let backend = Backend::start(&database, "127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let address = backend.address;
        let client = reqwest::Client::new();
        let health = client
            .get(format!("http://{address}/health"))
            .send()
            .await
            .unwrap();
        assert!(health.status().is_success());
        let status: serde_json::Value = client
            .get(format!("http://{address}/api/v1/status"))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(status["selected_league"].is_null());
        assert_eq!(status["exchange_pages"], 0);
        assert_eq!(status["exchange_markets"], 0);
        assert_eq!(status["ninja_snapshots"], 0);
        let store = SqliteStore::connect(&database).await.unwrap();
        assert!(store.economy_imports().await.unwrap().is_empty());
        let archive = serde_json::from_value(serde_json::json!({
            "name": "Test archive", "status": "completed", "rows": 0, "skipped": 0,
            "error": null, "start": null, "end": null, "league": "Test league",
            "progress": "Saved test dataset", "license": "poe.ninja CC BY-SA",
            "url": "https://poe.ninja/"
        }))
        .unwrap();
        store.save_economy_import(&archive, None).await.unwrap();
        backend.stop().await.unwrap();
        assert!(database.is_file());

        let restarted = Backend::start(&database, address).await.unwrap();
        assert!(
            client
                .get(format!("http://{address}/health"))
                .send()
                .await
                .unwrap()
                .status()
                .is_success()
        );
        let reopened = SqliteStore::connect(&database).await.unwrap();
        let archives = reopened.economy_imports().await.unwrap();
        assert_eq!(archives.len(), 1);
        assert_eq!(archives[0].name, "Test archive");
        assert_eq!(archives[0].status, "completed");
        restarted.stop().await.unwrap();
        let listener = tokio::net::TcpListener::bind(address).await.unwrap();
        drop(listener);
    }

    #[tokio::test]
    async fn occupied_port_fails_without_creating_a_database() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("faust.sqlite3");
        let result = Backend::start(&database, listener.local_addr().unwrap()).await;
        let error = result.err().expect("an occupied port must fail");
        assert!(error.to_string().contains("Close any other Faust"));
        assert!(!database.exists());
    }
}
