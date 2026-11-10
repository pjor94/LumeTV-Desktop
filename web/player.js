(function (global) {
  'use strict';

  const CODECS = Object.freeze({
    video: 'video/mp4; codecs="avc1.640028"',
    audio: 'audio/mp4; codecs="mp4a.40.2"',
  });
  const WEBM_CODECS = Object.freeze({
    video: 'video/webm; codecs="vp9"',
    audio: 'audio/webm; codecs="opus"',
  });
  const KEY_STATUSES = new Set(['usable', 'expired', 'released', 'output-restricted', 'output-downscaled', 'status-pending', 'internal-error']);

  function number(value, fallback = 0) {
    return typeof value === 'number' && Number.isFinite(value) ? value : fallback;
  }

  function sanitizedError(error) {
    return {
      code: number(error && error.code),
      category: number(error && error.category),
      severity: number(error && error.severity),
    };
  }

  function safeKeyStatuses(statuses) {
    const values = statuses instanceof Map ? Array.from(statuses.values()) : Object.values(statuses || {});
    return Array.from(new Set(values.filter(value => KEY_STATUSES.has(value)))).sort();
  }

  function encodeBody(body) {
    if (body == null) return undefined;
    const bytes = body instanceof ArrayBuffer
      ? new Uint8Array(body)
      : ArrayBuffer.isView(body)
        ? new Uint8Array(body.buffer, body.byteOffset, body.byteLength)
        : null;
    if (!bytes) throw new TypeError('Unsupported request body');
    let binary = '';
    for (let index = 0; index < bytes.length; index += 0x4000) {
      binary += String.fromCharCode.apply(null, bytes.subarray(index, index + 0x4000));
    }
    return global.btoa(binary);
  }

  function relayHeaders(headers) {
    const result = {};
    // Provider authentication stays in Rust. Only protocol headers are forwarded.
    for (const [name, value] of Object.entries(headers || {})) {
      if (/^(accept|content-type|range)$/i.test(name) && typeof value === 'string' && !/[\r\n]/.test(value)) {
        result[name] = value;
      }
    }
    return result;
  }

  function buildRelayRequest(uri, request, kind) {
    const payload = {
      kind,
      method: request.method || 'GET',
      headers: relayHeaders(request.headers),
    };
    if (kind === 'media') {
      const original = uri.startsWith('lume:') ? uri.slice(5) : uri;
      if (!/^https?:\/\//i.test(original)) throw new TypeError('Unsupported media scheme');
      payload.url = original;
    } else if (kind !== 'license') {
      throw new TypeError('Unsupported request kind');
    }
    const body = encodeBody(request.body);
    if (body !== undefined) payload.body_base64 = body;
    return payload;
  }

  class PlaybackEvidence {
    constructor(durationSeconds) {
      this.required = Math.max(1, number(durationSeconds, 60));
      this.movingSeconds = 0;
      this.current = null;
      this.wall = null;
      this.frames = 0;
      this.samples = 0;
    }

    observe(sample, wallSeconds) {
      const current = number(sample.current_time);
      const wall = number(wallSeconds);
      if (this.current !== null && this.wall !== null && !sample.paused && !sample.seeking) {
        const advanced = current - this.current;
        const elapsed = wall - this.wall;
        // Reject seeks, stalled samples, and very large sampling gaps.
        if (elapsed > 0 && elapsed <= 5 && advanced > 0.04 && advanced <= Math.max(2.5, elapsed * 2)) {
          this.movingSeconds += Math.min(advanced, elapsed);
        }
      }
      this.current = current;
      this.wall = wall;
      this.frames = Math.max(this.frames, number(sample.video_frames));
      this.samples += 1;
      return this.movingSeconds >= this.required && this.frames > 0 && sample.width > 0 && sample.height > 0;
    }
  }

  function makeRelayPlugin(shaka, apiFetch, kind) {
    return function (uri, request) {
      const abort = new AbortController();
      const started = performance.now();
      const promise = (async () => {
        try {
          const payload = buildRelayRequest(uri, request, kind);
          const response = await apiFetch('/api/fetch', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
            signal: abort.signal,
          });
          if (!response.ok) {
            throw new shaka.util.Error(shaka.util.Error.Severity.CRITICAL,
              shaka.util.Error.Category.NETWORK, shaka.util.Error.Code.BAD_HTTP_STATUS, response.status);
          }
          const originalUri = kind === 'media' ? payload.url : 'lume-license:license';
          const finalUri = kind === 'media' ? response.headers.get('X-Lume-Final-Url') || originalUri : originalUri;
          if (kind === 'media' && !/^https?:\/\//i.test(finalUri)) {
            throw new shaka.util.Error(shaka.util.Error.Severity.CRITICAL,
              shaka.util.Error.Category.NETWORK, shaka.util.Error.Code.HTTP_ERROR);
          }
          const headers = {};
          for (const name of ['content-type', 'content-length', 'content-range', 'accept-ranges']) {
            const value = response.headers.get(name);
            if (value !== null) headers[name] = value;
          }
          return {
            uri: finalUri,
            originalUri,
            data: await response.arrayBuffer(),
            headers,
            timeMs: performance.now() - started,
            fromCache: false,
          };
        } catch (error) {
          if (error instanceof shaka.util.Error) throw error;
          throw new shaka.util.Error(shaka.util.Error.Severity.CRITICAL,
            shaka.util.Error.Category.NETWORK,
            abort.signal.aborted ? shaka.util.Error.Code.OPERATION_ABORTED : shaka.util.Error.Code.HTTP_ERROR);
        }
      })();
      return new shaka.util.AbortableOperation(promise, () => {
        abort.abort();
        return Promise.resolve();
      });
    };
  }

  async function probeCapabilities(video, environment = global) {
    const result = {
      secure_context: environment.isSecureContext === true,
      eme_available: typeof environment.navigator.requestMediaKeySystemAccess === 'function',
      widevine_available: false,
      widevine_webm_available: false,
      requested_video_codec: CODECS.video,
      requested_audio_codec: CODECS.audio,
      requested_webm_video_codec: WEBM_CODECS.video,
      requested_webm_audio_codec: WEBM_CODECS.audio,
      mse_available: typeof environment.MediaSource !== 'undefined',
      avc_mse_supported: !!(environment.MediaSource && environment.MediaSource.isTypeSupported(CODECS.video)),
      aac_mse_supported: !!(environment.MediaSource && environment.MediaSource.isTypeSupported(CODECS.audio)),
      webm_mse_supported: !!(environment.MediaSource && environment.MediaSource.isTypeSupported(WEBM_CODECS.video) && environment.MediaSource.isTypeSupported(WEBM_CODECS.audio)),
      native_avc_supported: video.canPlayType(CODECS.video) !== '',
      native_aac_supported: video.canPlayType(CODECS.audio) !== '',
    };
    if (result.eme_available) {
      try {
        const access = await environment.navigator.requestMediaKeySystemAccess('com.widevine.alpha', [{
          initDataTypes: ['cenc'],
          audioCapabilities: [{ contentType: CODECS.audio }],
          videoCapabilities: [{ contentType: CODECS.video }],
          distinctiveIdentifier: 'optional',
          persistentState: 'optional',
          sessionTypes: ['temporary'],
        }]);
        result.widevine_available = access.keySystem === 'com.widevine.alpha';
      } catch (error) {
        result.eme_error_code = sanitizedError(error).code;
      }
      // A codec-specific EME rejection does not by itself establish an absent CDM.
      try {
        const access = await environment.navigator.requestMediaKeySystemAccess('com.widevine.alpha', [{
          initDataTypes: ['webm', 'cenc'],
          audioCapabilities: [{ contentType: WEBM_CODECS.audio }],
          videoCapabilities: [{ contentType: WEBM_CODECS.video }],
          distinctiveIdentifier: 'optional',
          persistentState: 'optional',
          sessionTypes: ['temporary'],
        }]);
        result.widevine_webm_available = access.keySystem === 'com.widevine.alpha';
      } catch (error) {
        result.webm_error_code = sanitizedError(error).code;
      }
    }
    return result;
  }

  async function waitForWidevine(initial, {
    probe,
    timeoutSeconds = 0,
    now = () => performance.now(),
    sleep = milliseconds => new Promise(resolve => global.setTimeout(resolve, milliseconds)),
    isCancelled = () => false,
    onWaiting = async () => {},
  }) {
    let capabilities = initial;
    const timeout = Math.min(300, Math.max(0, number(timeoutSeconds))) * 1000;
    const started = now();
    const result = status => ({ capabilities, status, elapsed_seconds: Math.max(0, now() - started) / 1000 });
    if (capabilities.widevine_available) return result('ready');
    if (!timeout || !capabilities.secure_context || !capabilities.eme_available ||
        !capabilities.avc_mse_supported || !capabilities.aac_mse_supported) return result('unavailable');
    await onWaiting();
    while (!isCancelled()) {
      const remaining = timeout - (now() - started);
      if (remaining <= 0) return result('timeout');
      await sleep(Math.min(2000, remaining));
      if (isCancelled()) return result('cancelled');
      capabilities = await probe();
      if (isCancelled()) return result('cancelled');
      if (now() - started > timeout) return result('timeout');
      if (capabilities.widevine_available) return result('ready');
    }
    return result('cancelled');
  }

  async function bootstrap() {
    const doc = global.document;
    const video = doc.getElementById('video');
    const status = doc.getElementById('status');
    const metrics = doc.getElementById('metrics');
    const startButton = doc.getElementById('start');
    const unmuteButton = doc.getElementById('unmute');
    const audioButton = doc.getElementById('verify-audio');
    let token = '';
    try { token = decodeURIComponent(global.location.hash.slice(1)); } catch (_) { /* invalid token */ }
    global.history.replaceState(null, '', global.location.pathname + global.location.search);
    if (!/^[A-Za-z0-9_-]{16,256}$/.test(token)) {
      status.textContent = 'Sessione non autorizzata: apri l’indirizzo generato dal comando Rust.';
      return;
    }

    function apiFetch(path, options = {}) {
      return global.fetch(path, {
        ...options,
        cache: 'no-store',
        credentials: 'same-origin',
        headers: { ...(options.headers || {}), 'X-Lume-Token': token },
      });
    }

    let events = Promise.resolve();
    function emit(event) {
      events = events.then(async () => {
        const response = await apiFetch('/api/event', {
          method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(event),
        });
        if (!response.ok) throw new Error('Diagnostic transport failure');
      }).catch(() => {
        status.textContent = 'Invio della diagnostica locale non riuscito.';
      });
      return events;
    }

    let player = null;
    let interval = null;
    let watchdog = null;
    let finished = false;
    let encrypted = false;
    let encryptedEvents = 0;
    let keyStatuses = [];
    let callbackFrames = 0;
    let audioVerified = false;
    let evidence = null;
    let session = null;

    function readKeyStatuses() {
      try { keyStatuses = player ? safeKeyStatuses(player.getKeyStatuses()) : []; } catch (_) { keyStatuses = []; }
      return keyStatuses;
    }

    function sample() {
      let quality = null;
      try { quality = video.getVideoPlaybackQuality ? video.getVideoPlaybackQuality() : null; } catch (_) { /* metric unavailable */ }
      const qualityFrames = quality ? number(quality.totalVideoFrames) : 0;
      const webkitFrames = number(video.webkitDecodedFrameCount);
      const frames = Math.max(qualityFrames, webkitFrames, callbackFrames);
      const value = {
        current_time: number(video.currentTime),
        video_frames: frames,
        dropped_frames: quality ? number(quality.droppedVideoFrames) : number(video.webkitDroppedFrameCount),
        video_frame_source: quality ? 'playback_quality' : webkitFrames ? 'webkit_decoded' : callbackFrames ? 'video_frame_callback' : 'unavailable',
        ready_state: number(video.readyState),
        width: number(video.videoWidth),
        height: number(video.videoHeight),
        muted: video.muted,
        paused: video.paused,
        seeking: video.seeking,
        encrypted,
        encrypted_events: encryptedEvents,
        key_statuses: readKeyStatuses(),
        audio_audibly_verified: audioVerified,
      };
      if (typeof video.webkitAudioDecodedByteCount === 'number') {
        value.audio_decoded_bytes = number(video.webkitAudioDecodedByteCount);
        value.audio_decode_metric_available = true;
      } else {
        value.audio_decode_metric_available = false;
      }
      return value;
    }

    async function complete(success, reason) {
      if (finished) return;
      finished = true;
      global.clearInterval(interval);
      global.clearTimeout(watchdog);
      const last = sample();
      status.textContent = success
        ? 'Prova video superata: avanzamento e frame osservati per la durata richiesta.'
        : reason === 'widevine_setup_timeout'
          ? 'Preparazione dei canali protetti non completata. Controlla la connessione e riapri l’app.'
          : 'Prova non superata. Consulta il rapporto diagnostico locale.';
      await emit({
        kind: 'complete', success, reason,
        moving_seconds: evidence ? evidence.movingSeconds : 0,
        required_seconds: evidence ? evidence.required : 0,
        samples: evidence ? evidence.samples : 0,
        ...last,
      });
    }

    function fail(error) {
      if (finished) return;
      emit({ kind: 'error', ...sanitizedError(error) });
      complete(false, 'player_error');
    }

    function updateAudioControls() {
      unmuteButton.textContent = video.muted ? 'Attiva audio' : 'Disattiva audio';
      audioButton.disabled = video.muted || video.paused || audioVerified;
    }

    async function play() {
      try {
        await video.play();
        startButton.hidden = true;
        updateAudioControls();
      } catch (error) {
        startButton.hidden = false;
        status.textContent = 'Il browser richiede un clic per avviare la diretta.';
        emit({ kind: 'autoplay_blocked', ...sanitizedError(error) });
      }
    }

    startButton.addEventListener('click', play);
    unmuteButton.addEventListener('click', () => {
      video.muted = !video.muted;
      updateAudioControls();
      emit({ kind: 'audio_muted_changed', muted: video.muted });
    });
    audioButton.addEventListener('click', () => {
      if (video.muted || video.paused) return;
      audioVerified = true;
      audioButton.textContent = 'Audio verificato manualmente';
      updateAudioControls();
      emit({ kind: 'audio_verified', audio_audibly_verified: true });
    });
    video.addEventListener('volumechange', updateAudioControls);
    video.addEventListener('pause', updateAudioControls);
    video.addEventListener('playing', () => {
      updateAudioControls();
      emit({ kind: 'playing', ...sample() });
    });
    video.addEventListener('encrypted', () => {
      encrypted = true;
      encryptedEvents += 1;
      emit({ kind: 'encrypted', encrypted: true, encrypted_events: encryptedEvents });
    });
    video.addEventListener('error', () => fail(video.error));
    if (typeof video.requestVideoFrameCallback === 'function') {
      const onFrame = (_, metadata) => {
        callbackFrames = Math.max(callbackFrames + 1, number(metadata.presentedFrames));
        if (!finished) video.requestVideoFrameCallback(onFrame);
      };
      video.requestVideoFrameCallback(onFrame);
    }

    global.addEventListener('unhandledrejection', event => { event.preventDefault(); fail(event.reason); });
    global.addEventListener('error', event => { event.preventDefault(); fail(event.error); });

    try {
      const response = await apiFetch('/api/session');
      if (!response.ok) throw { code: response.status };
      session = await response.json();
      if (!session || !session.channel || !session.stream || !/^https?:\/\//i.test(session.stream.url) ||
          !['relay', 'direct'].includes(session.mode)) throw { code: 0 };
      evidence = new PlaybackEvidence(session.duration_seconds);
      const setupSeconds = session.stream.drm
        ? Math.min(300, Math.max(0, number(session.drm_setup_timeout_seconds))) : 0;
      const playbackTimeout = Math.max(90000, evidence.required * 2000 + 30000);
      doc.getElementById('channel').textContent = session.channel.name + ' · modalità ' + session.mode;
      status.textContent = 'Verifica delle capacità video e Widevine…';
      watchdog = global.setTimeout(() => complete(false, 'timeout'), setupSeconds * 1000 + playbackTimeout);

      const shaka = global.shaka;
      if (!shaka) throw { code: 0 };
      if (shaka.log && shaka.log.setLevel) shaka.log.setLevel(shaka.log.Level.NONE);
      shaka.polyfill.installAll();
      await emit({ kind: 'started', stage: 'capabilities' });
      let capabilities = await probeCapabilities(video);
      capabilities.shaka_supported = shaka.Player.isBrowserSupported();
      await emit({ kind: 'capabilities', ...capabilities });
      if (finished) return;
      if (!capabilities.shaka_supported) return complete(false, 'browser_unsupported');
      if (session.stream.drm && !capabilities.widevine_available) {
        const setup = await waitForWidevine(capabilities, {
          timeoutSeconds: setupSeconds,
          probe: async () => ({ ...(await probeCapabilities(video)), shaka_supported: shaka.Player.isBrowserSupported() }),
          isCancelled: () => finished,
          onWaiting: async () => {
            status.textContent = 'Preparazione dei canali protetti al primo avvio. Attendi…';
            await emit({ kind: 'drm_setup', status: 'waiting', max_wait_seconds: setupSeconds });
          },
        });
        if (finished || setup.status === 'cancelled') return;
        capabilities = setup.capabilities;
        await emit({ kind: 'capabilities', ...capabilities });
        await emit({ kind: 'drm_setup', status: setup.status, elapsed_seconds: setup.elapsed_seconds });
        if (setup.status === 'timeout') return complete(false, 'widevine_setup_timeout');
      }
      if (session.stream.drm && !capabilities.widevine_available) {
        return complete(false, !capabilities.avc_mse_supported || !capabilities.aac_mse_supported
          ? 'codec_unavailable' : 'widevine_unavailable');
      }
      global.clearTimeout(watchdog);
      watchdog = global.setTimeout(() => complete(false, 'timeout'), playbackTimeout);

      shaka.net.NetworkingEngine.registerScheme('lume', makeRelayPlugin(shaka, apiFetch, 'media'));
      shaka.net.NetworkingEngine.registerScheme('lume-license', makeRelayPlugin(shaka, apiFetch, 'license'));
      player = new shaka.Player();
      await player.attach(video);
      player.addEventListener('error', event => fail(event.detail));
      player.addEventListener('keystatuschanged', () => emit({ kind: 'key_statuses', key_statuses: readKeyStatuses() }));
      player.getNetworkingEngine().registerRequestFilter((_, request) => {
        if (session.mode === 'relay') {
          request.uris = request.uris.map(uri => /^https?:\/\//i.test(uri) ? 'lume:' + uri : uri);
        }
      });
      if (session.stream.drm) {
        player.configure({ drm: { servers: { 'com.widevine.alpha': 'lume-license:license' } } });
      }
      video.muted = true;
      interval = global.setInterval(() => {
        if (finished) return;
        const value = sample();
        const success = evidence.observe(value, performance.now() / 1000);
        value.moving_seconds = evidence.movingSeconds;
        metrics.textContent = JSON.stringify(value, null, 2);
        emit({ kind: 'sample', ...value });
        if (success) complete(true, 'playback_observed');
      }, 1000);
      status.textContent = 'Caricamento della diretta…';
      await player.load(session.stream.url, undefined, session.stream.mime_type || undefined);
      if (finished) return;
      await emit({ kind: 'loaded', drm_configured: !!session.stream.drm });
      unmuteButton.disabled = false;
      status.textContent = 'Misurazione della riproduzione in corso…';
      await play();
    } catch (error) {
      fail(error);
    }
  }

  const api = { CODECS, WEBM_CODECS, sanitizedError, safeKeyStatuses, encodeBody, relayHeaders, buildRelayRequest, PlaybackEvidence, makeRelayPlugin, probeCapabilities, waitForWidevine };
  if (typeof module === 'object' && module.exports) module.exports = api;
  global.LumePlayback = api;
  if (global.document && global.document.documentElement.hasAttribute('data-lume-harness')) bootstrap().catch(() => {
    global.document.getElementById('status').textContent = 'Inizializzazione della diagnostica non riuscita.';
  });
})(typeof window === 'undefined' ? globalThis : window);
