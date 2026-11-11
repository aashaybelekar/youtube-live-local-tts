<h1 align="center">YouTube Live TTS (Native Rust Edition)</h1>

<p align="center">
  Super-lightweight, real-time Text-to-Speech for YouTube live streams running locally via <b>hexgrad/Kokoro-82M</b>.
</p>

---

## Highlights

- **Ultra-lightweight native binary**: Single executable (~116MB stripped binary with embedded web dashboard).
- **Multi-Hardware Acceleration**:
  - **Apple Silicon (MacBook MPU / Neural Engine & GPU)**: Native CoreML execution out-of-the-box.
  - **NVIDIA GPU**: CUDA execution provider.
  - **AMD GPU**: DirectML (Windows) & ROCm (Linux) execution providers.
  - **CPU (Universal)**: SIMD vector acceleration (AVX2 / ARM NEON).
- **Local Kokoro-82M TTS**: High-quality local neural speech synthesis with zero cloud API keys or fees.
- **Hardware-Direct Audio**: Direct PCM audio output using `rodio` (CoreAudio on macOS, WASAPI on Windows, ALSA on Linux).
- **Embedded Web Dashboard**: Fully reactive dashboard and WebSocket server running on `http://localhost:8000` with zero Node/Bun runtime overhead.
- **Full Moderation Controls**: Customizable `{author} says: {message}` format templates, per-user cooldowns, prefix filters (`!tts`), and length restrictions.
- **Instant Voice Cutoff & Fresh Stream Capture**: Immediately cuts audio on bot stop and discards all historical messages sent before the bot started.

---

## Quick Start

### 1. Run Directly

On **macOS (Apple Silicon MPU / Neural Engine)** or standard **CPU**:

```bash
cargo run --release
```

Or execute the compiled binary directly:

```bash
./target/release/youtube-live-tts
```

#### Hardware Acceleration Build Flags:

- **NVIDIA GPU (CUDA)**:
  ```bash
  cargo run --release --features cuda
  ```
- **AMD GPU (DirectML on Windows / ROCm on Linux)**:
  ```bash
  cargo run --release --features directml
  # Or on Linux ROCm:
  cargo run --release --features rocm
  ```

---

### 2. Connect to YouTube Stream

1. Open **[http://localhost:8000](http://localhost:8000)** in your browser or add it as an OBS Browser Source.
2. Enter your **YouTube live stream URL** or video ID.
3. Select your compute device (Auto, Apple Silicon Neural Engine, NVIDIA GPU, AMD GPU, or CPU).
4. Select your preferred Kokoro voice and press **Start Bot**.

---

## Running Tests

All features are covered by automated unit and integration tests:

```bash
cargo test
```

---

## Uninstalling & Cleanup

To clean build artifacts, downloaded models, or uninstall Homebrew packages installed for this project:

```bash
./scripts/uninstall.sh
```
