import test from 'node:test';
import assert from 'node:assert/strict';
import { StartupController } from '../src/startup';
import { WidevineSetupError } from '../src/widevine';
import type { Capabilities } from '../src/runtime';

const absent: Capabilities = {
    secure_context: true,
    eme_available: true,
    avc_mse_supported: true,
    aac_mse_supported: true,
    widevine_available: false,
};

test('an already usable component opens the app without requesting another download', async () => {
    const controller = new StartupController(
        async () => ({ ...absent, widevine_available: true }),
        async () => {
            throw Error('Unexpected download');
        },
    );
    await controller.start();
    assert.equal(controller.store.get().phase, 'ready');
});

test('concurrent startup attempts share one check and one preparation', async () => {
    let checks = 0,
        downloads = 0;
    let complete!: () => void;
    const done = new Promise<void>((resolve) => {
        complete = resolve;
    });
    const controller = new StartupController(
        async () => {
            checks++;
            return absent;
        },
        async (initial, options) => {
            downloads++;
            options.update(
                {
                    phase: 'downloading',
                    version: '',
                    installed_at_startup: false,
                    restart_required: false,
                    error: null,
                    elapsed_seconds: 3,
                },
                3,
            );
            await done;
            return { ...initial, widevine_available: true };
        },
    );
    const a = controller.start();
    const b = controller.start();
    assert.equal(a, b);
    await new Promise((resolve) => setTimeout(resolve, 0));
    assert.equal(controller.store.get().phase, 'preparing');
    assert.equal(controller.store.get().elapsed, 3);
    complete();
    await a;
    assert.equal(checks, 1);
    assert.equal(downloads, 1);
    assert.equal(controller.store.get().phase, 'ready');
});

test('a failed first installation stays blocked and can be retried', async () => {
    let attempts = 0;
    const controller = new StartupController(
        async () => absent,
        async (initial) => {
            if (++attempts === 1) throw new WidevineSetupError('widevine_service_error');
            return { ...initial, widevine_available: true };
        },
    );
    await controller.start();
    assert.equal(controller.store.get().phase, 'error');
    await controller.start();
    assert.equal(controller.store.get().phase, 'ready');
});

test('Linux waits for a real restart instead of marking installation as usable', async () => {
    const controller = new StartupController(
        async () => absent,
        async () => {
            throw new WidevineSetupError('widevine_restart');
        },
    );
    await controller.start();
    assert.equal(controller.store.get().phase, 'restart');
    assert.match(controller.store.get().detail, /chiudi completamente Lume e riaprila/);
});

test('missing codecs fail before a component download is attempted', async () => {
    const controller = new StartupController(
        async () => ({ ...absent, aac_mse_supported: false }),
        async () => {
            throw Error('Unexpected download');
        },
    );
    await controller.start();
    assert.equal(controller.store.get().phase, 'error');
    assert.equal(controller.store.get().title, 'Il motore video non è pronto.');
});
