pub mod config;
pub mod tts;
pub mod youtube;
pub mod server;

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex, RwLock};
use config::{AppConfig, BotStatus};
use server::{create_router, AppState};
use tts::TtsManager;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\x1b[1;36m========================================================\x1b[0m");
    println!("\x1b[1;32m       YouTube Live Kokoro TTS (Native Rust Edition)     \x1b[0m");
    println!("\x1b[1;36m========================================================\x1b[0m");

    println!("-> Initializing Kokoro-82M TTS engine...");
    let tts = TtsManager::new()
        .await
        .map_err(|e| format!("Initialization failed: {}", e))?;

    let backend = tts.active_backend();
    println!("-> Kokoro-82M TTS engine ready. Compute backend: \x1b[1;33m{}\x1b[0m", backend);

    let (tx, _) = broadcast::channel(100);

    let state = AppState {
        config: Arc::new(RwLock::new(AppConfig::default())),
        status: Arc::new(RwLock::new(BotStatus {
            backend,
            ..Default::default()
        })),
        history: Arc::new(RwLock::new(Vec::new())),
        cooldowns: Arc::new(RwLock::new(HashMap::new())),
        tts,
        tx,
        abort_handles: Arc::new(Mutex::new(Vec::new())),
    };

    let app = create_router(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8000").await?;
    println!("\n\x1b[1;32m[OK]\x1b[0m Dashboard running at: \x1b[1;34mhttp://localhost:8000\x1b[0m");
    println!("Open this URL in your browser or OBS Browser Source.\n");

    axum::serve(listener, app).await?;

    Ok(())
}
