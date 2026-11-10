const test = require('node:test');
const assert = require('node:assert/strict');
const { CODECS, WEBM_CODECS, sanitizedError, safeKeyStatuses, buildRelayRequest, PlaybackEvidence, makeRelayPlugin, probeCapabilities, waitForWidevine } = require('./player.js');

test('error events include only numeric metadata, never messages, URLs, data or tokens', () => {
  assert.deepEqual(sanitizedError({ code: 6007, category: 6, severity: 2, message: 'secret', data: ['https://secret'], token: 'secret' }),
    { code: 6007, category: 6, severity: 2 });
  assert.deepEqual(sanitizedError({ code: 'secret', category: Infinity }), { code: 0, category: 0, severity: 0 });
});

test('key status reporting strips key IDs and unexpected status text', () => {
  assert.deepEqual(safeKeyStatuses({ secretKeyId: 'usable', another: 'usable', third: 'https://secret' }), ['usable']);
  assert.deepEqual(safeKeyStatuses(new Map([['secret', 'expired'], ['other', 'usable']])), ['expired', 'usable']);
});

test('media relay preserves original URL and protocol headers, omitting credential headers', () => {
  const request = buildRelayRequest('lume:https://example.test/a/manifest.mpd', {
    method: 'GET', headers: { Range: 'bytes=0-99', Authorization: 'secret', Cookie: 'secret', 'User-Agent': 'provider', Accept: '*/*' },
  }, 'media');
  assert.deepEqual(request, { kind: 'media', method: 'GET', headers: { Range: 'bytes=0-99', Accept: '*/*' }, url: 'https://example.test/a/manifest.mpd' });
});

test('license requests never send a URL and encode exactly the view bytes of a binary challenge', () => {
  const backing = new Uint8Array([9, 0, 1, 255, 9]);
  const request = buildRelayRequest('lume-license:license', {
    method: 'POST', headers: { 'Content-Type': 'application/octet-stream', Authorization: 'secret' }, body: backing.subarray(1, 4),
  }, 'license');
  assert.equal(request.url, undefined);
  assert.equal(request.body_base64, 'AAH/');
  assert.deepEqual(request.headers, { 'Content-Type': 'application/octet-stream' });
});

test('a loaded manifest, static currentTime, missing frames, or seeking cannot pass playback', () => {
  const evidence = new PlaybackEvidence(3);
  const sample = { current_time: 0, video_frames: 0, paused: false, width: 1920, height: 1080 };
  assert.equal(evidence.observe(sample, 0), false);
  assert.equal(evidence.observe(sample, 10), false);
  assert.equal(evidence.observe({ ...sample, current_time: 100, seeking: true, video_frames: 20 }, 11), false);
  assert.equal(evidence.observe({ ...sample, current_time: 101 }, 12), false);
  assert.equal(evidence.observe({ ...sample, current_time: 102 }, 13), false);
  assert.equal(evidence.observe({ ...sample, current_time: 103 }, 14), true);
  const noFrames = new PlaybackEvidence(2);
  for (let i = 0; i < 5; i++) assert.equal(noFrames.observe({ ...sample, current_time: i }, i), false);
});

test('reproduction requires the full measured moving duration, excluding stalled intervals', () => {
  const evidence = new PlaybackEvidence(3);
  const sample = time => ({ current_time: time, video_frames: 100, paused: false, width: 1280, height: 720 });
  assert.equal(evidence.observe(sample(0), 0), false);
  assert.equal(evidence.observe(sample(1), 1), false);
  assert.equal(evidence.observe(sample(1), 2), false);
  assert.equal(evidence.observe(sample(2), 3), false);
  assert.equal(evidence.observe(sample(3), 4), true);
  assert.equal(evidence.movingSeconds, 3);
});

class ShakaError extends Error {
  static Severity = { CRITICAL: 2 };
  static Category = { NETWORK: 1 };
  static Code = { BAD_HTTP_STATUS: 1001, HTTP_ERROR: 1002, OPERATION_ABORTED: 7001 };
  constructor(severity, category, code) { super(); this.severity = severity; this.category = category; this.code = code; }
}
const fakeShaka = { util: { Error: ShakaError, AbortableOperation: class { constructor(promise, onAbort) { this.promise = promise; this.abort = onAbort; } } } };

test('relay response keeps the upstream final URI for relative manifest segments', async () => {
  const plugin = makeRelayPlugin(fakeShaka, async () => new Response(new Uint8Array([1, 2]), {
    headers: { 'X-Lume-Final-Url': 'https://cdn.test/redirected/master.m3u8', 'Content-Type': 'application/vnd.apple.mpegurl' },
  }), 'media');
  const response = await plugin('lume:https://origin.test/live', { method: 'GET', headers: {} }).promise;
  assert.equal(response.uri, 'https://cdn.test/redirected/master.m3u8');
  assert.equal(response.originalUri, 'https://origin.test/live');
  assert.deepEqual(Array.from(new Uint8Array(response.data)), [1, 2]);
  assert.equal(response.headers['x-lume-final-url'], undefined);
});

test('license response does not expose the upstream license URI', async () => {
  const plugin = makeRelayPlugin(fakeShaka, async () => new Response(new Uint8Array([1]), {
    headers: { 'X-Lume-Final-Url': 'https://license.test/private?token=secret' },
  }), 'license');
  const response = await plugin('lume-license:license', { method: 'POST', headers: {} }).promise;
  assert.equal(response.uri, 'lume-license:license');
  assert.equal(response.originalUri, 'lume-license:license');
});

test('relay cancellation aborts fetch and reports a numeric Shaka error', async () => {
  let signal;
  const plugin = makeRelayPlugin(fakeShaka, async (_, options) => {
    signal = options.signal;
    return new Promise((_, reject) => signal.addEventListener('abort', () => reject(new Error('private URL')), { once: true }));
  }, 'media');
  const operation = plugin('lume:https://example.test/manifest.mpd', { headers: {} });
  await operation.abort();
  assert.equal(signal.aborted, true);
  await assert.rejects(operation.promise, error => error.code === 7001 && error.message === '');
});

test('Widevine WebM availability is measured separately from unsupported AVC/AAC codecs', async () => {
  const requests = [];
  const environment = {
    isSecureContext: true,
    MediaSource: { isTypeSupported: codec => [WEBM_CODECS.video, WEBM_CODECS.audio].includes(codec) },
    navigator: { requestMediaKeySystemAccess: async (keySystem, configurations) => {
      requests.push({ keySystem, configurations });
      if (configurations[0].videoCapabilities[0].contentType === CODECS.video) {
        throw { code: 9, message: 'sensitive provider information must not survive' };
      }
      return { keySystem: 'com.widevine.alpha' };
    } },
  };
  const capabilities = await probeCapabilities({ canPlayType: () => '' }, environment);
  assert.equal(capabilities.widevine_available, false);
  assert.equal(capabilities.eme_error_code, 9);
  assert.equal(capabilities.avc_mse_supported, false);
  assert.equal(capabilities.aac_mse_supported, false);
  assert.equal(capabilities.webm_mse_supported, true);
  assert.equal(capabilities.widevine_webm_available, true);
  assert.equal(requests.length, 2);
  assert.deepEqual(requests[1].configurations[0].initDataTypes, ['webm', 'cenc']);
  assert.equal(requests[1].configurations[0].videoCapabilities[0].contentType, WEBM_CODECS.video);
  assert.equal(requests[1].configurations[0].audioCapabilities[0].contentType, WEBM_CODECS.audio);
  assert.ok(requests.every(request => request.keySystem === 'com.widevine.alpha'));
  assert.equal(JSON.stringify(capabilities).includes('sensitive'), false);
});

test('failed WebM and AVC requests preserve separate numeric errors without claiming CDM availability', async () => {
  const environment = {
    isSecureContext: true,
    MediaSource: { isTypeSupported: () => true },
    navigator: { requestMediaKeySystemAccess: async (_, configurations) => {
      throw { code: configurations[0].videoCapabilities[0].contentType === CODECS.video ? 9 : 18, message: 'private details' };
    } },
  };
  const capabilities = await probeCapabilities({ canPlayType: () => 'probably' }, environment);
  assert.equal(capabilities.widevine_available, false);
  assert.equal(capabilities.widevine_webm_available, false);
  assert.equal(capabilities.eme_error_code, 9);
  assert.equal(capabilities.webm_error_code, 18);
  assert.equal(JSON.stringify(capabilities).includes('private'), false);
});

const setupCapabilities = {
  secure_context: true, eme_available: true, avc_mse_supported: true,
  aac_mse_supported: true, widevine_available: false,
};

test('first-run setup waits for Widevine to become available and keeps the final capabilities', async () => {
  let time = 0;
  let probes = 0;
  let waiting = 0;
  const result = await waitForWidevine(setupCapabilities, {
    timeoutSeconds: 10, now: () => time, sleep: async ms => { time += ms; },
    onWaiting: async () => { waiting++; },
    probe: async () => ({ ...setupCapabilities, widevine_available: ++probes === 2 }),
  });
  assert.equal(result.status, 'ready');
  assert.equal(result.capabilities.widevine_available, true);
  assert.equal(result.elapsed_seconds, 4);
  assert.equal(waiting, 1);
});

test('first-run setup stops at the configured deadline if the component remains unavailable', async () => {
  let time = 0;
  const result = await waitForWidevine(setupCapabilities, {
    timeoutSeconds: 5, now: () => time, sleep: async ms => { time += ms; },
    probe: async () => setupCapabilities,
  });
  assert.equal(result.status, 'timeout');
  assert.equal(result.elapsed_seconds, 5);
  assert.equal(result.capabilities.widevine_available, false);
});

test('ready CDM, absent codecs or EME, insecure contexts and disabled setup skip polling', async () => {
  for (const [capabilities, timeoutSeconds, expected] of [
    [{ ...setupCapabilities, widevine_available: true }, 5, 'ready'],
    [{ ...setupCapabilities, avc_mse_supported: false }, 5, 'unavailable'],
    [{ ...setupCapabilities, aac_mse_supported: false }, 5, 'unavailable'],
    [{ ...setupCapabilities, eme_available: false }, 5, 'unavailable'],
    [{ ...setupCapabilities, secure_context: false }, 5, 'unavailable'],
    [setupCapabilities, 0, 'unavailable'],
  ]) {
    const result = await waitForWidevine(capabilities, {
      timeoutSeconds, probe: async () => { assert.fail('unexpected probe'); },
      onWaiting: async () => { assert.fail('unexpected waiting'); },
    });
    assert.equal(result.status, expected);
  }
});

test('closing the player during setup prevents further probes', async () => {
  let cancelled = false;
  const result = await waitForWidevine(setupCapabilities, {
    timeoutSeconds: 5, isCancelled: () => cancelled,
    sleep: async () => { cancelled = true; },
    probe: async () => { assert.fail('unexpected probe'); },
  });
  assert.equal(result.status, 'cancelled');
});
