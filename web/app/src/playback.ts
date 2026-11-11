import { api, apiFetch, bounded } from './api';
import { createStore } from './store';
import { prepareWidevine, widevineMessage } from './widevine';
import type { ShakaPlayer } from './runtime';
import type { Channel, PlaybackPhase, PlaybackState, Preferences, Session } from './types';

interface Dependencies {
    channel(id: string): Channel | undefined;
    preferences(): Preferences;
    played(id: string): Promise<unknown>;
    watch(): void;
    stopped(): void;
}
const initial: PlaybackState = {
    current: null,
    channel: null,
    phase: 'idle',
    title: '',
    detail: '',
    hasPicture: false,
    paused: true,
    muted: false,
    volume: 0.7,
    height: 0,
    audioTracks: [],
    textTracks: [],
    audio: '',
    text: 'off',
    widevine: null,
    setupElapsed: 0,
};

// The engine belongs to the application, not to a routed screen. React only
// subscribes to its presentation state and attaches the one persistent video.
export class PlaybackController {
    readonly store = createStore(initial);
    private video: HTMLVideoElement | null = null;
    private player: ShakaPlayer | null = null;
    private playerPromise: Promise<ShakaPlayer> | null = null;
    private session: Session | null = null;
    private pending = false;
    private generation = 0;
    private playedId: string | null = null;
    private loadController: AbortController | null = null;
    private sampleTimer?: ReturnType<typeof setInterval>;

    constructor(private readonly deps: Dependencies) {}

    attach(video: HTMLVideoElement) {
        this.video = video;
        const events = new AbortController();
        video.addEventListener(
            'playing',
            () => {
                if (this.pending || !this.session) return;
                this.store.set({
                    phase: 'playing',
                    hasPicture: true,
                    paused: false,
                    widevine: null,
                });
                this.emit({ ...this.sample(), kind: 'playing' });
                if (this.playedId !== this.session.playback_id) {
                    this.playedId = this.session.playback_id;
                    void this.deps.played(this.session.playback_id).catch(() => {});
                }
            },
            { signal: events.signal },
        );
        video.addEventListener(
            'pause',
            () => {
                this.store.set({
                    paused: true,
                    ...(!this.pending && this.store.get().phase === 'playing'
                        ? { phase: 'paused' as const }
                        : {}),
                });
            },
            { signal: events.signal },
        );
        video.addEventListener(
            'encrypted',
            () => this.emit({ kind: 'encrypted', encrypted: true, encrypted_events: 1 }),
            { signal: events.signal },
        );
        video.addEventListener(
            'volumechange',
            () => this.store.set({ volume: video.volume, muted: video.muted }),
            { signal: events.signal },
        );
        this.applyPreferences();
        return () => {
            events.abort();
            ++this.generation;
            this.loadController?.abort();
            clearInterval(this.sampleTimer);
            const player = this.player;
            this.player = null;
            this.playerPromise = null;
            this.video = null;
            if (player) void player.destroy().catch(() => {});
        };
    }

    private phase(phase: PlaybackPhase, title = '', detail = '') {
        this.store.set({ phase, title, detail });
    }

    private emit(event: Record<string, unknown>, session = this.session) {
        if (!session) return;
        void apiFetch('/api/event', {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
                'X-Lume-Playback-Id': session.playback_id,
            },
            body: JSON.stringify(event),
        }).catch(() => {});
    }

    private sample() {
        const video = this.video;
        const quality = video?.getVideoPlaybackQuality?.();
        const extra = video as
            | (HTMLVideoElement & {
                  webkitAudioDecodedByteCount?: number;
                  webkitDecodedFrameCount?: number;
              })
            | null;
        return {
            kind: 'sample',
            current_time: video?.currentTime || 0,
            video_frames: quality?.totalVideoFrames || extra?.webkitDecodedFrameCount || 0,
            dropped_frames: quality?.droppedVideoFrames || 0,
            width: video?.videoWidth || 0,
            height: video?.videoHeight || 0,
            ready_state: video?.readyState || 0,
            muted: video?.muted,
            paused: video?.paused,
            seeking: video?.seeking,
            ...(typeof extra?.webkitAudioDecodedByteCount === 'number'
                ? {
                      audio_decoded_bytes: extra.webkitAudioDecodedByteCount,
                      audio_decode_metric_available: true,
                  }
                : {}),
        };
    }

    private ensurePlayer() {
        if (this.playerPromise) return this.playerPromise;
        this.playerPromise = (async () => {
            const shaka = window.shaka;
            if (!shaka || !this.video) throw new Error('player_missing');
            shaka.log?.setLevel(shaka.log.Level.NONE);
            shaka.polyfill.installAll();
            const relay = (path: string, options: RequestInit) => {
                const headers = new Headers(options.headers);
                headers.set('X-Lume-Playback-Id', this.session?.playback_id || '');
                return apiFetch(path, { ...options, headers });
            };
            shaka.net.NetworkingEngine.registerScheme(
                'lume',
                window.LumePlayback.makeRelayPlugin(shaka, relay, 'media'),
            );
            shaka.net.NetworkingEngine.registerScheme(
                'lume-license',
                window.LumePlayback.makeRelayPlugin(shaka, relay, 'license'),
            );
            const player = new shaka.Player();
            this.player = player;
            await player.attach(this.video);
            player.getNetworkingEngine().registerRequestFilter((_, request) => {
                request.uris = request.uris.map((uri) =>
                    /^https?:\/\//i.test(uri) ? 'lume:' + uri : uri,
                );
            });
            player.configure({
                preferredAudioLanguage: 'it',
                streaming: { bufferingGoal: 15, rebufferingGoal: 2 },
                manifest: { retryParameters: { maxAttempts: 3, baseDelay: 800, backoffFactor: 2 } },
            });
            player.addEventListener('error', (event) => {
                const error = event.detail as { code?: number } | undefined;
                if (this.pending || [7000, 7001].includes(error?.code || 0)) return;
                this.emit({ kind: 'error', ...window.LumePlayback.sanitizedError(error) });
                this.phase(
                    'error',
                    'La diretta si è interrotta.',
                    'Controlla la connessione e riprova. Puoi anche scegliere un altro canale.',
                );
            });
            player.addEventListener('buffering', (event) => {
                if (event.buffering && this.store.get().phase === 'playing' && !this.pending)
                    this.phase(
                        'buffering',
                        'Riprendiamo il filo.',
                        'Stiamo recuperando la diretta…',
                    );
                else if (!event.buffering && this.store.get().phase === 'buffering')
                    this.phase('playing');
            });
            player.addEventListener('keystatuschanged', () =>
                this.emit({
                    kind: 'key_statuses',
                    key_statuses: window.LumePlayback.safeKeyStatuses(player.getKeyStatuses()),
                }),
            );
            for (const event of ['trackschanged', 'variantchanged', 'adaptation'])
                player.addEventListener(event, () => this.updateTracks());
            return player;
        })().catch((error) => {
            this.playerPromise = null;
            throw error;
        });
        return this.playerPromise;
    }

    applyPreferences() {
        const preferences = this.deps.preferences();
        if (this.video) {
            this.video.volume = preferences.volume;
            this.video.muted = preferences.muted;
        }
        this.store.set({ volume: preferences.volume, muted: preferences.muted });
        this.player?.configure({
            abr: { enabled: true },
            restrictions: {
                maxHeight: preferences.quality === 'auto' ? Infinity : Number(preferences.quality),
            },
        });
    }
    private updateTracks() {
        if (!this.player) return;
        const variants = this.player.getVariantTracks();
        const active = variants.find((t) => t.active);
        const audioTracks = Array.from(
            new Map(variants.filter((t) => t.language).map((t) => [t.language, t])).values(),
        );
        const textTracks = this.player.getTextTracks();
        const text = textTracks.find((t) => t.active);
        this.store.set({
            height: active?.height || 0,
            audioTracks,
            textTracks,
            audio: active?.language || '',
            text: this.player.isTextTrackVisible() && text ? String(text.id) : 'off',
        });
    }

    async load(id: string) {
        const channel = this.deps.channel(id);
        if (!channel || !this.video) return;
        if (
            id === this.store.get().current &&
            ['playing', 'paused', 'buffering'].includes(this.store.get().phase)
        ) {
            this.deps.watch();
            return;
        }
        const generation = ++this.generation;
        let pendingSession: Session | null = null;
        this.loadController?.abort();
        this.loadController = new AbortController();
        clearInterval(this.sampleTimer);
        this.pending = true;
        this.playedId = null;
        this.store.set({
            current: id,
            channel,
            hasPicture: false,
            widevine: null,
            setupElapsed: 0,
            height: 0,
        });
        this.video.pause();
        this.deps.watch();
        this.phase(
            'loading',
            `Sintonizzazione su ${channel.name}…`,
            'Ancora un istante, il tuo canale sta arrivando.',
        );
        try {
            const signal = this.loadController.signal;
            const session = await bounded(
                api<Session>('/api/play', { channel_id: id }, { signal }),
                60_000,
                signal,
            );
            if (generation !== this.generation) return;
            pendingSession = session;
            const player = await bounded(this.ensurePlayer(), 10_000, signal);
            if (generation !== this.generation) return;
            await bounded(player.unload(), 10_000, signal);
            if (generation !== this.generation) return;
            this.session = session;
            let capabilities = await bounded(
                window.LumePlayback.probeCapabilities(this.video),
                12_000,
                signal,
            );
            if (generation !== this.generation) return;
            this.emit({ kind: 'capabilities', ...capabilities });
            if (!capabilities.avc_mse_supported || !capabilities.aac_mse_supported)
                throw { code: 'codec_unavailable' };
            if (session.stream.drm && !capabilities.widevine_available) {
                this.emit({ kind: 'drm_setup', status: 'waiting' });
                capabilities = await prepareWidevine(capabilities, {
                    signal,
                    probe: () => window.LumePlayback.probeCapabilities(this.video!),
                    update: (widevine, setupElapsed) => {
                        if (generation !== this.generation) return;
                        const [title, detail] = widevineMessage(widevine);
                        this.store.set({
                            widevine: widevine || {
                                phase: 'unavailable',
                                version: '',
                                installed_at_startup: false,
                                restart_required: false,
                                error: null,
                                elapsed_seconds: 0,
                            },
                            setupElapsed,
                            title,
                            detail,
                        });
                    },
                });
                if (generation !== this.generation) return;
                this.emit({ kind: 'capabilities', ...capabilities });
                this.emit({
                    kind: 'drm_setup',
                    status: 'ready',
                    elapsed_seconds: this.store.get().setupElapsed,
                });
                this.store.set({ widevine: null });
            }
            player.configure({
                drm: {
                    servers: session.stream.drm
                        ? { 'com.widevine.alpha': 'lume-license:license' }
                        : {},
                },
            });
            this.applyPreferences();
            await bounded(
                player.load(session.stream.url, undefined, session.stream.mime_type || undefined),
                45_000,
                signal,
            );
            if (generation !== this.generation) return;
            this.emit({ kind: 'loaded', drm_configured: !!session.stream.drm });
            this.pending = false;
            this.applyPreferences();
            await this.resume();
            if (generation !== this.generation) return;
            this.updateTracks();
            this.sampleTimer = setInterval(() => {
                if (this.session && this.video && this.video.readyState >= 2)
                    this.emit(this.sample());
            }, 2000);
        } catch (error) {
            const failure = error as { name?: string; code?: string };
            if (generation !== this.generation || failure.name === 'AbortError') return;
            ++this.generation;
            this.loadController?.abort();
            this.pending = false;
            void this.player?.unload().catch(() => {});
            this.emit(
                { kind: 'error', ...window.LumePlayback.sanitizedError(error) },
                pendingSession,
            );
            if (failure.code === 'widevine_restart')
                this.phase(
                    'restart',
                    'Widevine è installato. Riapri Lume.',
                    'Su Linux il componente viene caricato all’avvio. Chiudi completamente Lume e riaprila per guardare i canali protetti.',
                );
            else if (failure.code === 'widevine_service_error')
                this.phase(
                    'error',
                    'Download di Widevine non riuscito.',
                    'Controlla la connessione e premi Riprova. Gli altri canali restano disponibili.',
                );
            else if (failure.code === 'widevine_unavailable')
                this.phase(
                    'error',
                    'I canali protetti non sono ancora pronti.',
                    'Verifica la connessione, poi riprova o riapri Lume.',
                );
            else if (failure.code === 'codec_unavailable')
                this.phase(
                    'error',
                    'Questo formato non è disponibile.',
                    'Il motore video di questa versione non supporta il formato del canale.',
                );
            else
                this.phase(
                    'error',
                    'Questo canale si fa attendere.',
                    'La sorgente non è al momento disponibile. Riprova tra poco o scegli un altro canale.',
                );
        }
    }
    async resume() {
        try {
            await this.video?.play();
        } catch {
            this.phase(
                'interaction',
                'Il tuo canale è pronto.',
                'Premi Avvia per iniziare la visione con audio.',
            );
        }
    }
    togglePause() {
        if (this.video?.paused) void this.resume();
        else this.video?.pause();
    }
    volume(volume: number, muted: boolean) {
        if (this.video) {
            this.video.volume = volume;
            this.video.muted = muted;
        }
        this.store.set({ volume, muted });
    }
    audio(language: string) {
        this.player?.selectAudioLanguage(language);
        this.store.set({ audio: language });
    }
    text(id: string) {
        if (!this.player) return;
        const track = this.player.getTextTracks().find((t) => String(t.id) === id);
        if (track) this.player.selectTextTrack(track);
        this.player.setTextTrackVisibility(!!track);
        this.store.set({ text: id });
    }
    goLive() {
        try {
            if (this.video && this.player) {
                this.video.currentTime = this.player.seekRange().end - 0.3;
                void this.resume();
            }
        } catch {
            /* Not seekable yet. */
        }
    }
    async stop() {
        const generation = ++this.generation;
        const playbackId = this.session?.playback_id;
        this.loadController?.abort();
        this.pending = false;
        clearInterval(this.sampleTimer);
        void api('/api/stop', { playback_id: playbackId || null }).catch(() => {});
        this.video?.pause();
        await this.player?.unload().catch(() => {});
        if (generation !== this.generation) return;
        this.session = null;
        this.store.set({
            ...initial,
            volume: this.video?.volume || 0,
            muted: this.video?.muted || false,
        });
        this.deps.stopped();
    }
}
