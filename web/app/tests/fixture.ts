import { readFileSync } from 'node:fs';
import type { Page } from '@playwright/test';
import type { Channel, Preferences, WidevineStatus } from '../src/types';
import type { UpdateStatus } from '../src/updates';

export const updateStatus: UpdateStatus = {
    current_version: '0.1.0',
    phase: 'up_to_date',
    available_version: null,
    notes: '',
    downloaded: 0,
    total: 0,
    error: null,
    settings: { automatic: true, preview: true },
    last_checked: 1794416400,
    next_check_at: 9999999999,
    install_mode: 'replace',
    last_install_failed: false,
};

export const now = new Date('2026-11-11T18:00:00+01:00').getTime();
export const channels: Channel[] = JSON.parse(
    readFileSync(new URL('../../../resources/catalog-legacy.json', import.meta.url), 'utf8'),
);
export const defaultPreferences: Preferences = {
    favorites: ['rai1', 'canale5'],
    recent: ['rai1'],
    reminders: [],
    theme: 'dark',
    volume: 0.7,
    muted: true,
    quality: 'auto',
    zap_favorites_only: false,
};
export const catalog = {
    updated_at: now,
    loading: false,
    error: null,
    revision: 'fixture',
    next_retry_at: 0,
};
export const guide = {
    programmes: Object.fromEntries(
        channels.map((c) => [
            c.id,
            [
                {
                    title: `In onda su ${c.name}`,
                    description:
                        'Programmazione di prova per verificare la continuità dell’interfaccia.',
                    start: now - 1_800_000,
                    end: now + 1_800_000,
                },
                {
                    title: `A seguire su ${c.name}`,
                    description: '',
                    start: now + 1_800_000,
                    end: now + 7_200_000,
                },
            ],
        ]),
    ),
    logos: {},
    updated_at: now,
    loading: false,
    next_retry_at: 0,
};

export async function fixture(
    page: Page,
    {
        widevine = true,
        startupWidevine = true,
        status,
        delayPlay = false,
        updates,
        radioChannels = [],
        radioTitle = '',
        continuousRadio = false,
    }: {
        widevine?: boolean;
        startupWidevine?: boolean | (() => boolean);
        status?: Partial<WidevineStatus> | (() => Partial<WidevineStatus>);
        delayPlay?: boolean;
        radioChannels?: Channel[];
        radioTitle?: string;
        continuousRadio?: boolean;
        updates?:
            | Partial<UpdateStatus>
            | ((path: string, body: Record<string, unknown>) => Partial<UpdateStatus>);
    } = {},
) {
    let preferences = structuredClone(defaultPreferences);
    const allChannels = [...channels, ...radioChannels];
    const wav = Buffer.alloc(44 + 32000);
    wav.write('RIFF', 0);
    wav.writeUInt32LE(wav.length - 8, 4);
    wav.write('WAVEfmt ', 8);
    wav.writeUInt32LE(16, 16);
    wav.writeUInt16LE(1, 20);
    wav.writeUInt16LE(1, 22);
    wav.writeUInt32LE(16000, 24);
    wav.writeUInt32LE(32000, 28);
    wav.writeUInt16LE(2, 32);
    wav.writeUInt16LE(16, 34);
    wav.write('data', 36);
    wav.writeUInt32LE(32000, 40);
    await page.route('**/audio/fixture.wav', (route) =>
        route.fulfill({ contentType: 'audio/wav', body: wav }),
    );
    const calls: string[] = [];
    const plays: string[] = [];
    await page.clock.setFixedTime(now);
    await page.route('**/api/**', async (route) => {
        const request = route.request();
        const path = new URL(request.url()).pathname;
        calls.push(path);
        const body = request.method() === 'POST' ? request.postDataJSON() : {};
        let data: unknown;
        if (path === '/api/bootstrap')
            data = { channels: allChannels, preferences, catalog, version: '0.1.0', os: 'linux' };
        else if (path.startsWith('/api/updates'))
            data = {
                ...updateStatus,
                ...(typeof updates === 'function' ? updates(path, body) : updates),
            };
        else if (path.startsWith('/api/catalog') || path === '/api/radios/refresh')
            data = { ...catalog, channels: allChannels };
        else if (path.startsWith('/api/guide')) data = guide;
        else if (path === '/api/preferences') {
            if (body.toggle_favorite) {
                const id = body.toggle_favorite;
                preferences.favorites = preferences.favorites.includes(id)
                    ? preferences.favorites.filter((x) => x !== id)
                    : [...preferences.favorites, id];
            }
            if (body.toggle_reminder) {
                const r = body.toggle_reminder;
                const existing = preferences.reminders.some(
                    (x) => x.channel_id === r.channel_id && x.start === r.start,
                );
                preferences.reminders = existing
                    ? preferences.reminders.filter(
                          (x) => x.channel_id !== r.channel_id || x.start !== r.start,
                      )
                    : [...preferences.reminders, r];
            }
            if (body.clear_recent) preferences.recent = [];
            for (const key of [
                'theme',
                'quality',
                'volume',
                'muted',
                'zap_favorites_only',
            ] as const)
                if (body[key] !== undefined) Object.assign(preferences, { [key]: body[key] });
            data = preferences;
        } else if (path === '/api/played') {
            const id = String(body.playback_id).replace(/^test-/, '');
            if (allChannels.some((c) => c.id === id))
                preferences.recent = [id, ...preferences.recent.filter((x) => x !== id)].slice(
                    0,
                    40,
                );
            data = preferences;
        } else if (path === '/api/radio/status')
            data = { playback_id: `test-${plays.at(-1)}`, audio: { now_playing: radioTitle } };
        else if (path === '/api/play') {
            plays.push(body.channel_id);
            if (delayPlay && body.channel_id === 'rai1')
                await new Promise((resolve) => setTimeout(resolve, 600));
            data = {
                playback_id: `test-${body.channel_id}`,
                channel: allChannels.find((c) => c.id === body.channel_id),
                stream: {
                    url:
                        continuousRadio && radioChannels.some((c) => c.id === body.channel_id)
                            ? '/audio/fixture.wav'
                            : 'lume:https://fixture.invalid/live.mpd',
                    continuous:
                        continuousRadio && radioChannels.some((c) => c.id === body.channel_id),
                    drm:
                        radioChannels.some((c) => c.id === body.channel_id) ||
                        body.channel_id === 'rai1'
                            ? null
                            : {},
                },
            };
        } else if (path.startsWith('/api/widevine'))
            data = {
                phase: 'downloading',
                version: '',
                installed_at_startup: false,
                restart_required: false,
                error: null,
                elapsed_seconds: 0,
                ...(typeof status === 'function' ? status() : status),
            };
        else data = {};
        await route.fulfill({ json: data });
    });
    await page.route('**/vendor/shaka-player.compiled.js', (route) =>
        route.fulfill({
            contentType: 'text/javascript',
            body: `
    window.testPlayer = { attachments: 0, loads: 0, unloads: 0, destroyed: 0 };
    const paused = new WeakMap();
    Object.defineProperty(HTMLMediaElement.prototype, 'paused', { get() { return paused.get(this) ?? true; } });
    HTMLMediaElement.prototype.play = async function () { paused.set(this, false); this.dispatchEvent(new Event('playing')); };
    HTMLMediaElement.prototype.pause = function () { paused.set(this, true); this.dispatchEvent(new Event('pause')); };
    window.shaka = {
      polyfill: { installAll() {} }, net: { NetworkingEngine: { registerScheme() {} } },
      Player: class {
        constructor() { this.handlers = {}; window.testPlayer.emit = (name, event) => this.handlers[name]?.(event); }
        async attach(video) { this.video = video; window.testPlayer.attachments++; }
        async unload() { window.testPlayer.unloads++; }
        async destroy() { window.testPlayer.destroyed++; }
        async load() { window.testPlayer.loads++; }
        configure() {} addEventListener(name, fn) { this.handlers[name] = fn; }
        getNetworkingEngine() { return { registerRequestFilter() {} }; }
        getVariantTracks() { return [{id: 1, language: 'it', height: 720, active: true}]; }
        getTextTracks() { return []; } isTextTrackVisible() { return false; }
        getKeyStatuses() { return {}; } selectAudioLanguage() {} setTextTrackVisibility() {} seekRange() { return {end: 10}; }
      }
    };`,
        }),
    );
    const player = readFileSync(new URL('../../player.js', import.meta.url), 'utf8');
    await page.route('**/player.js', (route) =>
        route.fulfill({
            contentType: 'text/javascript',
            body:
                player +
                `\nwindow.testWidevineReady = ${widevine}; window.testStartupWidevineReady = ${typeof startupWidevine === 'function' ? startupWidevine() : startupWidevine}; LumePlayback.probeCapabilities = async video => ({ secure_context:true, eme_available:true, avc_mse_supported:true, aac_mse_supported:true, widevine_available:video.id === 'video' ? window.testWidevineReady : window.testStartupWidevineReady });`,
        }),
    );
    await page.goto('/#fixture-session-token-123456');
    return { calls, plays, preferences: () => preferences };
}
