let ws = null;
let isRunning = false;

// DOM Elements
const connectionBadge = document.getElementById('connectionBadge');
const toggleBotBtn = document.getElementById('toggleBotBtn');
const statStatus = document.getElementById('statStatus');
const statQueue = document.getElementById('statQueue');
const statRead = document.getElementById('statRead');
const statBackend = document.getElementById('statBackend');
const chatHistory = document.getElementById('chatHistory');

// Form Elements
const videoIdInput = document.getElementById('videoId');
const deviceIdSelect = document.getElementById('deviceId');
const voiceIdSelect = document.getElementById('voiceId');
const ttsPrefixInput = document.getElementById('ttsPrefix');
const speedRange = document.getElementById('speedRange');
const speedVal = document.getElementById('speedVal');
const volumeRange = document.getElementById('volumeRange');
const volVal = document.getElementById('volVal');
const cooldownInput = document.getElementById('cooldown');
const maxLengthInput = document.getElementById('maxLength');
const ttsTemplateInput = document.getElementById('ttsTemplate');

// Connect WebSocket
function connectWebSocket() {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  const wsUrl = `${protocol}//${window.location.host}/ws`;

  ws = new WebSocket(wsUrl);

  ws.onopen = () => {
    console.log('[WS] Connected to Rust backend');
  };

  ws.onmessage = (event) => {
    try {
      const msg = JSON.parse(event.data);
      handleWsEvent(msg);
    } catch (e) {
      console.error('[WS] Parse error', e);
    }
  };

  ws.onclose = () => {
    console.log('[WS] Disconnected, retrying in 3s...');
    setTimeout(connectWebSocket, 3000);
  };
}

function handleWsEvent(evt) {
  if (evt.event === 'connected') {
    updateStatus(evt.data.status);
    if (evt.data.history) {
      chatHistory.innerHTML = '';
      evt.data.history.forEach(addChatMessage);
    }
  } else if (evt.event === 'status') {
    updateStatus(evt.data);
  } else if (evt.event === 'queue_update') {
    statQueue.textContent = evt.data.size;
  } else if (evt.event === 'message_read') {
    statRead.textContent = evt.data.total;
  } else if (evt.event === 'new_message') {
    addChatMessage(evt.data);
  }
}

function updateStatus(status) {
  if (!status) return;

  if (status.running !== undefined) {
    isRunning = status.running;
    toggleBotBtn.textContent = isRunning ? 'Stop Bot' : 'Start Bot';
    toggleBotBtn.className = isRunning ? 'btn btn-outline' : 'btn btn-primary';
    statStatus.textContent = isRunning ? (status.connected ? 'Listening' : 'Connecting...') : 'Idle';
  }

  if (status.connected !== undefined) {
    connectionBadge.textContent = status.connected ? 'Connected' : 'Disconnected';
    connectionBadge.className = status.connected ? 'badge badge-connected' : 'badge badge-disconnected';
  }

  if (status.queue_size !== undefined) {
    statQueue.textContent = status.queue_size;
  }

  if (status.messages_read !== undefined) {
    statRead.textContent = status.messages_read;
  }

  if (status.backend) {
    statBackend.textContent = status.backend;
  }
}

function addChatMessage(msg) {
  const empty = chatHistory.querySelector('.empty-state');
  if (empty) empty.remove();

  const item = document.createElement('div');
  item.className = `chat-item ${msg.was_read ? 'was-spoken' : ''}`;

  let roleClass = '';
  if (msg.is_owner) roleClass = 'owner';
  else if (msg.is_moderator) roleClass = 'moderator';

  item.innerHTML = `
    <div class="chat-meta">
      <span class="chat-author ${roleClass}">${escapeHtml(msg.author)}</span>
      <span class="chat-time">${escapeHtml(msg.timestamp || '')}</span>
    </div>
    <div class="chat-msg">${escapeHtml(msg.message)}</div>
  `;

  chatHistory.prepend(item);

  // Keep max 50 in DOM
  while (chatHistory.children.length > 50) {
    chatHistory.removeChild(chatHistory.lastChild);
  }
}

function escapeHtml(str) {
  if (!str) return '';
  return str.replace(/[&<>"']/g, m => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'
  })[m]);
}

// REST API calls
async function fetchConfig() {
  try {
    const res = await fetch('/api/config');
    if (!res.ok) return;
    const cfg = await res.json();
    videoIdInput.value = cfg.video_id || '';
    if (cfg.device && deviceIdSelect) {
      deviceIdSelect.value = cfg.device;
    }
    voiceIdSelect.value = cfg.voice_id || 'af_bella';
    ttsPrefixInput.value = cfg.tts_prefix || '';
    speedRange.value = cfg.speed || 1.0;
    speedVal.textContent = cfg.speed || 1.0;
    volumeRange.value = Math.round((cfg.volume ?? 1.0) * 100);
    volVal.textContent = Math.round((cfg.volume ?? 1.0) * 100);
    cooldownInput.value = cfg.cooldown_seconds ?? 10;
    maxLengthInput.value = cfg.max_message_length ?? 150;
    ttsTemplateInput.value = cfg.tts_template || '{author} says: {message}';
  } catch (e) {
    console.error('Failed to load config', e);
  }
}

async function fetchHardware() {
  try {
    const res = await fetch('/api/hardware');
    if (!res.ok) return;
    const data = await res.json();
    if (data.hardware && data.hardware.length > 0 && deviceIdSelect) {
      deviceIdSelect.innerHTML = '';
      data.hardware.forEach(h => {
        const opt = document.createElement('option');
        opt.value = h.id;
        opt.textContent = `${h.name} ${h.is_available ? '✓' : '(Not installed)'}`;
        deviceIdSelect.appendChild(opt);
      });
    }
  } catch (e) {
    console.warn('Failed to load hardware options', e);
  }
}

async function saveConfig(e) {
  if (e) e.preventDefault();
  const config = {
    video_id: videoIdInput.value.trim(),
    device: deviceIdSelect ? deviceIdSelect.value : 'auto',
    voice_id: voiceIdSelect.value || 'af_bella',
    tts_prefix: ttsPrefixInput.value.trim(),
    speed: parseFloat(speedRange.value) || 1.0,
    volume: (parseFloat(volumeRange.value) || 100) / 100.0,
    cooldown_seconds: parseInt(cooldownInput.value, 10) || 10,
    max_message_length: parseInt(maxLengthInput.value, 10) || 150,
    tts_template: ttsTemplateInput.value.trim() || '{author} says: {message}',
  };

  try {
    const res = await fetch('/api/config', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(config),
    });

    if (!res.ok) {
      const errText = await res.text();
      alert('Failed to save config: ' + errText);
      return false;
    }

    const data = await res.json();
    if (data.success) {
      // Re-query status to update active backend display
      const stRes = await fetch('/api/status');
      if (stRes.ok) {
        updateStatus(await stRes.json());
      }
      if (e) {
        alert('Configuration saved!');
      }
      return true;
    }
  } catch (e) {
    alert('Failed to save config: ' + e);
    return false;
  }
}

async function toggleBot() {
  const endpoint = isRunning ? '/api/stop' : '/api/start';
  if (!isRunning && !videoIdInput.value.trim()) {
    alert('Please enter a YouTube video URL or ID.');
    return;
  }
  
  // Auto-save config first
  const saved = await saveConfig();
  if (saved === false) {
    return;
  }

  try {
    const res = await fetch(endpoint, { method: 'POST' });
    if (!res.ok) {
      const errText = await res.text();
      alert('Action failed: ' + errText);
      return;
    }
    const data = await res.json();
    if (data.error) {
      alert(data.error);
    }
  } catch (e) {
    alert('Action failed: ' + e);
  }
}

async function testVoice() {
  try {
    const res = await fetch('/api/test-voice', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        voice_id: voiceIdSelect.value,
        speed: parseFloat(speedRange.value) || 1.0,
        volume: (parseFloat(volumeRange.value) || 100) / 100.0,
        text: 'Testing hardware accelerated Kokoro TTS!'
      })
    });
    if (!res.ok) {
      const errText = await res.text();
      alert('Test voice error: ' + errText);
    }
  } catch (e) {
    console.error('Voice test failed', e);
  }
}

// Initial Boot
window.addEventListener('DOMContentLoaded', async () => {
  await fetchHardware();
  await fetchConfig();
  connectWebSocket();
});
