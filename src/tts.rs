use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Mutex as TokioMutex;
use kokoro_micro::TtsEngine;
use rodio::{OutputStream, Sink, buffer::SamplesBuffer};
use serde::{Deserialize, Serialize};

pub const KOKORO_SAMPLE_RATE: u32 = 24000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareInfo {
    pub id: String,
    pub name: String,
    pub is_available: bool,
    pub description: String,
}

#[derive(Clone)]
pub struct TtsManager {
    engine: Arc<TokioMutex<TtsEngine>>,
    current_sink: Arc<Mutex<Option<Arc<Sink>>>>,
    is_stopped: Arc<AtomicBool>,
}

impl TtsManager {
    /// Initializes Kokoro TTS (caches model in ~/.cache/k automatically if not present).
    pub async fn new() -> Result<Self, String> {
        let engine = TtsEngine::new()
            .await
            .map_err(|e| format!("Failed to initialize Kokoro TTS engine: {}", e))?;
        Ok(Self {
            engine: Arc::new(TokioMutex::new(engine)),
            current_sink: Arc::new(Mutex::new(None)),
            is_stopped: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Returns a list of hardware acceleration backends (NVIDIA GPU, AMD GPU, Apple MPU, CPU).
    pub fn get_available_hardware() -> Vec<HardwareInfo> {
        let mut list = Vec::new();

        list.push(HardwareInfo {
            id: "auto".to_string(),
            name: "Auto (Best Available)".to_string(),
            is_available: true,
            description: "Automatically uses GPU/MPU acceleration when present, otherwise CPU".to_string(),
        });

        // Apple Silicon M-series MPU / Neural Engine (CoreML)
        let coreml_available = cfg!(target_os = "macos") && cfg!(target_arch = "aarch64");
        list.push(HardwareInfo {
            id: "coreml".to_string(),
            name: "Apple Silicon Neural Engine / MPU (CoreML)".to_string(),
            is_available: coreml_available,
            description: "Hardware acceleration via Apple M-series Neural Engine & GPU".to_string(),
        });

        // NVIDIA GPU (CUDA)
        let cuda_available = cfg!(feature = "cuda");
        list.push(HardwareInfo {
            id: "cuda".to_string(),
            name: "NVIDIA GPU (CUDA)".to_string(),
            is_available: cuda_available,
            description: "Hardware acceleration on NVIDIA GeForce / RTX GPUs".to_string(),
        });

        // AMD GPU (DirectML on Windows, ROCm on Linux)
        let directml_available = cfg!(feature = "directml") || cfg!(target_os = "windows");
        let rocm_available = cfg!(feature = "rocm");
        list.push(HardwareInfo {
            id: "amd".to_string(),
            name: "AMD GPU (DirectML / ROCm)".to_string(),
            is_available: directml_available || rocm_available,
            description: "Hardware acceleration on AMD Radeon RX GPUs & APUs".to_string(),
        });

        // CPU
        list.push(HardwareInfo {
            id: "cpu".to_string(),
            name: "CPU (Multi-threaded SIMD)".to_string(),
            is_available: true,
            description: "Runs on host CPU with AVX2/NEON vector instructions".to_string(),
        });

        list
    }

    /// Returns a human-readable name of the active compute backend.
    pub fn active_backend(&self) -> String {
        if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
            "Apple Silicon Neural Engine (CoreML)".to_string()
        } else if cfg!(feature = "cuda") {
            "NVIDIA GPU (CUDA)".to_string()
        } else if cfg!(feature = "directml") {
            "AMD GPU (DirectML)".to_string()
        } else if cfg!(feature = "rocm") {
            "AMD GPU (ROCm)".to_string()
        } else {
            "CPU (Multi-threaded)".to_string()
        }
    }

    /// Reconfigures compute target (e.g. CPU, GPU, Auto).
    pub async fn set_device(&self, device_name: &str) -> Result<String, String> {
        let dev = match device_name.to_lowercase().as_str() {
            "cpu" => kokoro_micro::Device::Cpu,
            "gpu" | "cuda" | "nvidia" => kokoro_micro::Device::Gpu,
            _ => kokoro_micro::Device::Auto,
        };
        let engine = self.engine.lock().await;
        let _ = engine.reload_on_device(dev);
        Ok(self.active_backend())
    }

    /// Stops any currently playing audio immediately and prevents further playback.
    pub fn stop_audio(&self) {
        self.is_stopped.store(true, Ordering::SeqCst);
        if let Ok(mut lock) = self.current_sink.lock() {
            if let Some(sink) = lock.take() {
                sink.stop();
            }
        }
    }

    /// Resets the stop flag so new audio can be spoken.
    pub fn reset_stop(&self) {
        self.is_stopped.store(false, Ordering::SeqCst);
    }

    /// Synthesizes text into 24kHz f32 PCM audio samples.
    pub async fn synthesize(
        &self,
        text: &str,
        voice: Option<&str>,
        speed: f32,
        volume: f32,
    ) -> Result<Vec<f32>, String> {
        let trimmed = text.trim();
        if trimmed.is_empty() || self.is_stopped.load(Ordering::SeqCst) {
            return Ok(Vec::new());
        }

        let engine = self.engine.lock().await;
        if self.is_stopped.load(Ordering::SeqCst) {
            return Ok(Vec::new());
        }

        // speed clamped between 0.5 and 2.0
        let clamped_speed = speed.clamp(0.5, 2.0);
        // gain clamped between 0.0 and 2.0
        let clamped_gain = volume.clamp(0.0, 2.0);

        engine
            .synthesize_with_options(trimmed, voice, clamped_speed, clamped_gain, None)
            .map_err(|e| format!("Kokoro synthesis error: {}", e))
    }

    /// Synthesizes text and plays it, listening for stop signals to terminate immediately.
    pub async fn speak(
        &self,
        text: &str,
        voice: Option<&str>,
        speed: f32,
        volume: f32,
    ) -> Result<(), String> {
        if self.is_stopped.load(Ordering::SeqCst) {
            return Ok(());
        }

        let samples = self.synthesize(text, voice, speed, volume).await?;
        if samples.is_empty() || self.is_stopped.load(Ordering::SeqCst) {
            return Ok(());
        }

        let current_sink = self.current_sink.clone();
        let is_stopped = self.is_stopped.clone();

        tokio::task::spawn_blocking(move || {
            let (_stream, stream_handle) = OutputStream::try_default()
                .map_err(|e| format!("Audio device error: {}", e))?;

            let sink = Arc::new(
                Sink::try_new(&stream_handle)
                    .map_err(|e| format!("Failed to create audio sink: {}", e))?,
            );

            {
                let mut cur = current_sink.lock().unwrap();
                if is_stopped.load(Ordering::SeqCst) {
                    return Ok(());
                }
                *cur = Some(sink.clone());
            }

            let buffer = SamplesBuffer::new(1, KOKORO_SAMPLE_RATE, samples);
            sink.append(buffer);

            // Play while checking cancellation periodically
            while !sink.empty() {
                if is_stopped.load(Ordering::SeqCst) {
                    sink.stop();
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }

            // Clear active sink reference
            {
                let mut cur = current_sink.lock().unwrap();
                *cur = None;
            }

            Ok::<(), String>(())
        })
        .await
        .map_err(|e| format!("Playback join error: {}", e))??;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_tts_initialization_and_synthesis() {
        let tts = TtsManager::new().await;
        assert!(tts.is_ok(), "Kokoro engine should initialize successfully");

        let manager = tts.unwrap();
        let samples = manager
            .synthesize("Test", Some("af_bella"), 1.0, 1.0)
            .await;
        assert!(samples.is_ok(), "Synthesis should produce audio samples");

        let data = samples.unwrap();
        assert!(!data.is_empty(), "Generated audio samples should not be empty");
    }

    #[tokio::test]
    async fn test_empty_text_returns_empty_samples() {
        let tts = TtsManager::new().await;
        assert!(tts.is_ok());

        let manager = tts.unwrap();
        let samples = manager.synthesize("   ", None, 1.0, 1.0).await.unwrap();
        assert!(samples.is_empty());
    }

    #[tokio::test]
    async fn test_stop_audio_flag_prevents_synthesis() {
        let tts = TtsManager::new().await.unwrap();
        tts.stop_audio();

        // While stopped, synthesize immediately returns empty
        let samples = tts.synthesize("Hello", None, 1.0, 1.0).await.unwrap();
        assert!(samples.is_empty(), "Stopped TTS should return empty samples");

        // Reset and check it synthesizes again
        tts.reset_stop();
        let samples_after_reset = tts.synthesize("Hello", None, 1.0, 1.0).await.unwrap();
        assert!(!samples_after_reset.is_empty(), "After reset, TTS should synthesize normally");
    }

    #[test]
    fn test_available_hardware_backends_detection() {
        let backends = TtsManager::get_available_hardware();
        assert!(!backends.is_empty(), "Should return hardware options");

        let has_auto = backends.iter().any(|b| b.id == "auto" && b.is_available);
        assert!(has_auto, "Auto backend should always be available");

        let has_cpu = backends.iter().any(|b| b.id == "cpu" && b.is_available);
        assert!(has_cpu, "CPU backend should always be available");

        let has_amd = backends.iter().any(|b| b.id == "amd");
        assert!(has_amd, "AMD GPU option should be present in hardware list");

        let has_nvidia = backends.iter().any(|b| b.id == "cuda");
        assert!(has_nvidia, "NVIDIA GPU option should be present in hardware list");

        let has_apple = backends.iter().any(|b| b.id == "coreml");
        assert!(has_apple, "Apple Silicon option should be present in hardware list");
    }
}
