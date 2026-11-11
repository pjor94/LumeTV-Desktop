import test from 'node:test';
import assert from 'node:assert/strict';
import { prepareWidevine } from '../src/widevine';
import type { Capabilities } from '../src/runtime';
import type { WidevineStatus } from '../src/types';
import type { api } from '../src/api';
const absent: Capabilities = {
    secure_context: true,
    eme_available: true,
    avc_mse_supported: true,
    aac_mse_supported: true,
    widevine_available: false,
};
const installed: WidevineStatus = {
    phase: 'installed',
    version: '4.10.3112.0',
    installed_at_startup: false,
    restart_required: false,
    error: null,
    elapsed_seconds: 1,
};
const request = (value: WidevineStatus) => (async () => value) as typeof api;

test('an installed component is not reported ready until EME succeeds', async () => {
    let clock = 0;
    let probes = 0;
    let updates = 0;
    const result = await prepareWidevine(absent, {
        signal: new AbortController().signal,
        request: request(installed),
        probe: async () => ({ ...absent, widevine_available: ++probes === 2 }),
        update: () => {
            updates++;
        },
        now: () => clock,
        sleep: async (ms) => {
            clock += ms;
        },
    });
    assert.equal(result.widevine_available, true);
    assert.equal(probes, 2);
    assert.ok(updates >= 2);
});
test('Linux asks for a restart only when the native monitor reports a new installation', async () => {
    await assert.rejects(
        prepareWidevine(absent, {
            signal: new AbortController().signal,
            request: request({ ...installed, restart_required: true }),
            probe: async () => absent,
            update() {},
        }),
        { code: 'widevine_restart' },
    );
    const result = await prepareWidevine(absent, {
        signal: new AbortController().signal,
        request: request({ ...installed, restart_required: true }),
        probe: async () => ({ ...absent, widevine_available: true }),
        update() {},
    });
    assert.equal(result.widevine_available, true);
});
test('a genuine updater error offers retry without waiting five minutes', async () => {
    await assert.rejects(
        prepareWidevine(absent, {
            signal: new AbortController().signal,
            request: request({ ...installed, phase: 'error', error: 'check_error' }),
            probe: async () => absent,
            update() {},
        }),
        { code: 'widevine_service_error' },
    );
});
test('changing channel cancels preparation without further readiness checks', async () => {
    const controller = new AbortController();
    let probes = 0;
    let clock = 0;
    await assert.rejects(
        prepareWidevine(absent, {
            signal: controller.signal,
            request: request({ ...installed, phase: 'downloading' }),
            probe: async () => {
                probes++;
                return absent;
            },
            update() {},
            now: () => clock,
            sleep: async (ms) => {
                clock += ms;
                controller.abort();
            },
        }),
        { name: 'AbortError' },
    );
    assert.equal(probes, 1);
});
test('an already installed but unusable component times out instead of looping over restart instructions', async () => {
    let clock = 0;
    await assert.rejects(
        prepareWidevine(absent, {
            signal: new AbortController().signal,
            request: request({ ...installed, installed_at_startup: true }),
            probe: async () => absent,
            update() {},
            timeout: 2000,
            now: () => clock,
            sleep: async (ms) => {
                clock += ms;
            },
        }),
        { code: 'widevine_unavailable' },
    );
});
