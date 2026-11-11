use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use axum::{
    extract::{
        rejection::JsonRejection,
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
        State,
    },
    http::{header, HeaderMap, StatusCode, Uri},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::{broadcast, mpsc, Mutex, RwLock};
use tokio::task::AbortHandle;
use tower_http::cors::CorsLayer;

use crate::config::{extract_video_id, AppConfig, BotStatus, ChatMessage};
use crate::tts::TtsManager;
use crate::youtube::process_chat_item;

#[derive(RustEmbed)]
#[folder = "ui/"]
pub struct Assets;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<RwLock<AppConfig>>,
    pub status: Arc<RwLock<BotStatus>>,
    pub history: Arc<RwLock<Vec<ChatMessage>>>,
    pub cooldowns: Arc<RwLock<HashMap<String, Instant>>>,
    pub tts: TtsManager,
    pub tx: broadcast::Sender<String>,
    pub abort_handles: Arc<Mutex<Vec<AbortHandle>>>,
}

pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/api/status", get(get_status))
        .route("/api/config", get(get_config).post(update_config))
        .route("/api/start", post(start_bot))
        .route("/api/stop", post(stop_bot))
        .route("/api/voices", get(get_voices))
        .route("/api/hardware", get(get_hardware))
        .route("/api/history", get(get_history))
        .route("/api/test-voice", post(test_voice))
        .route("/ws", get(ws_handler))
        .fallback(static_handler)
        .layer(CorsLayer::permissive())
        .with_state(state)
}

// Static Assets Handler (Serves Embedded UI)
async fn static_handler(uri: Uri) -> impl IntoResponse {
    let mut path = uri.path().trim_start_matches('/').to_string();
    if path.is_empty() {
        path = "index.html".to_string();
    }

    match Assets::get(&path) {
        Some(content) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            let mut headers = HeaderMap::new();
            headers.insert(header::CONTENT_TYPE, mime.as_ref().parse().unwrap());
            (StatusCode::OK, headers, content.data).into_response()
        }
        None => match Assets::get("index.html") {
            Some(content) => {
                let mut headers = HeaderMap::new();
                headers.insert(header::CONTENT_TYPE, "text/html".parse().unwrap());
                (StatusCode::OK, headers, content.data).into_response()
            }
            None => (StatusCode::NOT_FOUND, "Not Found").into_response(),
        },
    }
}

// REST Endpoints
async fn get_status(State(state): State<AppState>) -> Json<BotStatus> {
    let mut status = state.status.read().await.clone();
    status.backend = state.tts.active_backend();
    Json(status)
}

async fn get_config(State(state): State<AppState>) -> Json<AppConfig> {
    let config = state.config.read().await;
    Json(config.clone())
}

async fn update_config(
    State(state): State<AppState>,
    payload: Result<Json<AppConfig>, JsonRejection>,
) -> (StatusCode, Json<serde_json::Value>) {
    let Json(new_config) = match payload {
        Ok(cfg) => cfg,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "success": false, "error": err.body_text() })),
            );
        }
    };

    let device = new_config.device.clone();
    {
        let mut config = state.config.write().await;
        *config = new_config;
    }
    // Apply device preference to Kokoro TTS engine
    let backend_name = state.tts.set_device(&device).await.unwrap_or_else(|_| "CPU".to_string());
    {
        let mut st = state.status.write().await;
        st.backend = backend_name;
    }

    (StatusCode::OK, Json(json!({ "success": true })))
}

#[derive(Debug, Deserialize)]
struct TestVoiceRequest {
    voice_id: Option<String>,
    speed: Option<f32>,
    volume: Option<f32>,
    text: Option<String>,
}

async fn test_voice(
    State(state): State<AppState>,
    Json(payload): Json<TestVoiceRequest>,
) -> Json<serde_json::Value> {
    let text = payload
        .text
        .unwrap_or_else(|| "This is a test of Kokoro TTS in native Rust.".to_string());
    let voice = payload.voice_id.clone();
    let speed = payload.speed.unwrap_or(1.0);
    let volume = payload.volume.unwrap_or(1.0);

    state.tts.reset_stop();
    let tts = state.tts.clone();
    tokio::spawn(async move {
        if let Err(e) = tts.speak(&text, voice.as_deref(), speed, volume).await {
            eprintln!("[TTS] Test voice error: {}", e);
        }
    });

    Json(json!({ "success": true }))
}

async fn get_voices() -> Json<serde_json::Value> {
    let voices = vec![
        json!({ "id": "af_bella", "name": "Bella (American Female)" }),
        json!({ "id": "af_sarah", "name": "Sarah (American Female)" }),
        json!({ "id": "af_heart", "name": "Heart (Warm Female)" }),
        json!({ "id": "af_nicole", "name": "Nicole (Whisper/Soft Female)" }),
        json!({ "id": "af_sky", "name": "Sky (Clear Female)" }),
        json!({ "id": "am_adam", "name": "Adam (American Male)" }),
        json!({ "id": "am_michael", "name": "Michael (Deep Male)" }),
        json!({ "id": "am_eric", "name": "Eric (American Male)" }),
        json!({ "id": "am_echo", "name": "Echo (Neutral Male)" }),
        json!({ "id": "bf_emma", "name": "Emma (British Female)" }),
        json!({ "id": "bf_isabella", "name": "Isabella (British Female)" }),
        json!({ "id": "bm_george", "name": "George (British Male)" }),
        json!({ "id": "bm_lewis", "name": "Lewis (British Male)" }),
    ];
    Json(json!({ "voices": voices }))
}

async fn get_hardware() -> Json<serde_json::Value> {
    let hardware = TtsManager::get_available_hardware();
    Json(json!({ "hardware": hardware }))
}

async fn get_history(State(state): State<AppState>) -> Json<Vec<ChatMessage>> {
    let history = state.history.read().await;
    Json(history.clone())
}

async fn start_bot(State(state): State<AppState>) -> Json<serde_json::Value> {
    // 1. Drain and abort any previously active tasks to ensure zero ghost listeners
    {
        let mut handles = state.abort_handles.lock().await;
        for h in handles.drain(..) {
            h.abort();
        }
    }

    let config = state.config.read().await.clone();
    let video_id = extract_video_id(&config.video_id);
    if video_id.is_empty() {
        return Json(json!({ "error": "Missing YouTube URL or video ID" }));
    }

    // Reset stop state on TTS manager
    state.tts.reset_stop();

    // Record exact bot start time in UTC to ignore prior messages
    let bot_start_time = Utc::now();

    {
        let mut status = state.status.write().await;
        status.running = true;
        status.connected = false;
        status.messages_read = 0;
        status.queue_size = 0;
        status.backend = state.tts.active_backend();

        let _ = state.tx.send(
            json!({
                "event": "status",
                "data": status.clone()
            })
            .to_string(),
        );
    }

    // Channel for audio queue (max 100 queued spoken messages)
    let (queue_tx, mut queue_rx) = mpsc::channel::<(String, String)>(100);

    // Speaker Loop
    let tts = state.tts.clone();
    let status_clone = state.status.clone();
    let tx_clone = state.tx.clone();
    let voice_id = config.voice_id.clone();
    let speed = config.speed;
    let volume = config.volume;

    let speaker_task = tokio::spawn(async move {
        while let Some((author, text)) = queue_rx.recv().await {
            if !status_clone.read().await.running {
                break;
            }

            {
                let mut st = status_clone.write().await;
                st.queue_size = queue_rx.len();
                let _ = tx_clone.send(
                    json!({
                        "event": "queue_update",
                        "data": { "size": st.queue_size }
                    })
                    .to_string(),
                );
            }

            if let Err(e) = tts.speak(&text, Some(&voice_id), speed, volume).await {
                eprintln!("[Speaker] Speech error: {}", e);
            } else {
                let mut st = status_clone.write().await;
                st.messages_read += 1;
                let _ = tx_clone.send(
                    json!({
                        "event": "message_read",
                        "data": { "author": author, "total": st.messages_read }
                    })
                    .to_string(),
                );
            }
        }
    });

    // YouTube Live Listener Loop
    let state_clone = state.clone();
    let listener_task = tokio::spawn(async move {
        run_listener(state_clone, video_id, queue_tx, bot_start_time).await;
    });

    // Store actual task handles so stop_bot cleanly aborts each one
    {
        let mut handles = state.abort_handles.lock().await;
        handles.push(speaker_task.abort_handle());
        handles.push(listener_task.abort_handle());
    }

    Json(json!({ "success": true }))
}

async fn stop_bot(State(state): State<AppState>) -> Json<serde_json::Value> {
    // 1. Mark status as stopped immediately so loops exit on next tick
    {
        let mut status = state.status.write().await;
        status.running = false;
        status.connected = false;
        status.queue_size = 0;
    }

    // 2. Abort all background task handles directly (both listener and speaker)
    {
        let mut handles = state.abort_handles.lock().await;
        for h in handles.drain(..) {
            h.abort();
        }
    }

    // 3. Immediately terminate any voices currently playing
    state.tts.stop_audio();

    // 4. Broadcast status and queue updates to WebSocket clients
    let status = state.status.read().await.clone();
    let _ = state.tx.send(
        json!({
            "event": "status",
            "data": status
        })
        .to_string(),
    );
    let _ = state.tx.send(
        json!({
            "event": "queue_update",
            "data": { "size": 0 }
        })
        .to_string(),
    );

    Json(json!({ "success": true }))
}

async fn run_listener(
    state: AppState,
    video_id: String,
    queue_tx: mpsc::Sender<(String, String)>,
    bot_start_time: chrono::DateTime<Utc>,
) {
    use youtube_chat::live_chat::LiveChatClientBuilder;

    let (item_tx, mut item_rx) = mpsc::channel::<youtube_chat::item::ChatItem>(100);

    let builder = match LiveChatClientBuilder::new().url(format!("https://www.youtube.com/watch?v={}", video_id)) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[YouTube] Invalid URL or ID: {:?}", e);
            let mut st = state.status.write().await;
            st.connected = false;
            let _ = state.tx.send(json!({ "event": "status", "data": st.clone() }).to_string());
            return;
        }
    };

    let item_tx_clone = item_tx.clone();
    let mut client = builder
        .on_chat(move |chat_item| {
            let _ = item_tx_clone.try_send(chat_item);
        })
        .build();

    if let Err(e) = client.start().await {
        eprintln!("[YouTube] Failed to start live chat client: {:?}", e);
        let mut st = state.status.write().await;
        st.connected = false;
        let _ = state.tx.send(json!({ "event": "status", "data": st.clone() }).to_string());
        return;
    }

    {
        let mut st = state.status.write().await;
        st.connected = true;
        let _ = state.tx.send(
            json!({ "event": "status", "data": st.clone() }).to_string(),
        );
    }

    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(1500));
    let mut seen_ids: HashSet<String> = HashSet::new();

    loop {
        // Exit loop if status is no longer running
        if !state.status.read().await.running {
            let _ = client.stop().await;
            break;
        }

        tokio::select! {
            _ = ticker.tick() => {
                if !state.status.read().await.running {
                    let _ = client.stop().await;
                    break;
                }
                client.execute().await;
            }
            Some(item) = item_rx.recv() => {
                if !state.status.read().await.running {
                    let _ = client.stop().await;
                    break;
                }

                // Deduplicate messages by their unique YouTube Chat ID
                if !item.id.is_empty() && !seen_ids.insert(item.id.clone()) {
                    continue; // Skip duplicate message
                }

                if seen_ids.len() > 1000 {
                    seen_ids.clear();
                }

                let config = state.config.read().await.clone();
                let mut cooldowns = state.cooldowns.write().await;
                let (chat_msg, speech) = process_chat_item(&item, &config, &mut cooldowns, Some(bot_start_time));

                // Add to history (keep max 100)
                {
                    let mut history = state.history.write().await;
                    history.push(chat_msg.clone());
                    if history.len() > 100 {
                        history.remove(0);
                    }
                }

                // Broadcast new message to UI
                let _ = state.tx.send(json!({
                    "event": "new_message",
                    "data": chat_msg
                }).to_string());

                // If message should be spoken, queue it
                if let Some(text_to_speak) = speech {
                    if let Ok(()) = queue_tx.try_send((chat_msg.author, text_to_speak)) {
                        let mut st = state.status.write().await;
                        st.queue_size += 1;
                        let _ = state.tx.send(json!({
                            "event": "queue_update",
                            "data": { "size": st.queue_size }
                        }).to_string());
                    }
                }
            }
        }
    }

    // Ensure disconnected status on exit
    let _ = client.stop().await;
    {
        let mut st = state.status.write().await;
        st.connected = false;
        let _ = state.tx.send(json!({ "event": "status", "data": st.clone() }).to_string());
    }
}

// WebSocket handler
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let mut rx = state.tx.subscribe();

    // Initial sync
    let mut status = state.status.read().await.clone();
    status.backend = state.tts.active_backend();
    let history = state.history.read().await.clone();
    let init_event = json!({
        "event": "connected",
        "data": {
            "status": status,
            "history": history
        }
    });

    if let Ok(msg) = serde_json::to_string(&init_event) {
        if socket.send(WsMessage::Text(msg.into())).await.is_err() {
            return;
        }
    }

    loop {
        tokio::select! {
            Ok(msg) = rx.recv() => {
                if socket.send(WsMessage::Text(msg.into())).await.is_err() {
                    break;
                }
            }
            res = socket.recv() => {
                if res.is_none() {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn create_test_state() -> AppState {
        let tts = TtsManager::new().await.expect("TTS init in tests");
        let (tx, _) = broadcast::channel(100);
        AppState {
            config: Arc::new(RwLock::new(AppConfig::default())),
            status: Arc::new(RwLock::new(BotStatus::default())),
            history: Arc::new(RwLock::new(Vec::new())),
            cooldowns: Arc::new(RwLock::new(HashMap::new())),
            tts,
            tx,
            abort_handles: Arc::new(Mutex::new(Vec::new())),
        }
    }

    #[tokio::test]
    async fn test_get_status_endpoint() {
        let state = create_test_state().await;
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/status")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_get_config_and_update_config() {
        let state = create_test_state().await;
        let app = create_router(state);

        // GET config
        let res1 = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/config")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res1.status(), StatusCode::OK);

        // POST config
        let update_body = json!({
            "video_id": "test_video",
            "voice_id": "af_heart",
            "tts_prefix": "!read",
            "speed": 1.1,
            "volume": 0.8,
            "cooldown_seconds": 15,
            "max_message_length": 200,
            "tts_template": "[{author}]: {message}",
            "device": "auto"
        });

        let res2 = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/config")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(update_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res2.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_update_config_handles_partial_payload() {
        let state = create_test_state().await;
        let app = create_router(state);

        let partial_body = json!({
            "video_id": "dQw4w9WgXcQ"
        });

        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/config")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(partial_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_get_voices_endpoint() {
        let state = create_test_state().await;
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/voices")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_get_hardware_endpoint() {
        let state = create_test_state().await;
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/hardware")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_static_embedded_assets() {
        let state = create_test_state().await;
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "text/html"
        );
    }

    #[tokio::test]
    async fn test_stop_bot_resets_status_and_stops_audio() {
        let state = create_test_state().await;

        // Simulate running bot with queue
        {
            let mut st = state.status.write().await;
            st.running = true;
            st.connected = true;
            st.queue_size = 5;
        }

        let app = create_router(state.clone());

        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/stop")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::OK);

        let status = state.status.read().await;
        assert!(!status.running, "Bot should not be running after stop");
        assert!(!status.connected, "Bot should not be connected after stop");
        assert_eq!(status.queue_size, 0, "Queue size should be reset to 0");
    }

    #[tokio::test]
    async fn test_start_and_stop_bot_cleans_handles_and_no_duplicates() {
        let state = create_test_state().await;

        // Set valid video id in config
        {
            let mut cfg = state.config.write().await;
            cfg.video_id = "dQw4w9WgXcQ".to_string();
        }

        let app = create_router(state.clone());

        // 1. Start bot
        let res_start = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/start")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res_start.status(), StatusCode::OK);

        // Check handles were stored
        {
            let handles = state.abort_handles.lock().await;
            assert_eq!(handles.len(), 2, "Should store listener and speaker handles");
        }

        // 2. Stop bot
        let res_stop = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/stop")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res_stop.status(), StatusCode::OK);

        // Check handles were drained and aborted
        {
            let handles = state.abort_handles.lock().await;
            assert_eq!(handles.len(), 0, "Handles should be empty after stop");
            assert!(!state.status.read().await.running);
        }

        // 3. Start bot again -> must cleanly spawn without leaving old tasks behind
        let res_restart = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/start")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res_restart.status(), StatusCode::OK);

        {
            let handles = state.abort_handles.lock().await;
            assert_eq!(handles.len(), 2, "Should have exactly 2 fresh handles, no duplicates");
        }
    }
}
