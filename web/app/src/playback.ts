import { api, apiFetch, bounded } from './api';
import { createStore } from './store';
import { prepareWidevine, widevineMessage } from './widevine';
import { RecoveryController } from './recovery';
import { isRadio } from './domain';
import type { ImportJob, PersonalSource } from './personal';
import type { ShakaPlayer } from './runtime';
import type { Channel, PlaybackPhase, PlaybackState, Preferences, Session } from './types';

interface Dependencies {
    channel(id: string): Channel | undefined | Promise<Channel | undefined>;
    preferences(): Preferences;
    changedPreferences?(patch: Partial<Preferences>): Promise<unknown>;
    played(id: string): Promise<unknown>;
    watch(): void;
    stopped(): void;
}
const initial: PlaybackState = {
    mode: 'live',
    contentTitle: '',
    position: 0,
    duration: 0,
    seekable: false,
    ended: false,
    nowPlaying: '',
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
    private wantsToPlay = false;
    private needsReload = false;
    private refreshedSource = '';
    private lookupGeneration = 0;
    private target: { mode?: string; event?: string; position?: number } = {};
    private watchQueue = Promise.resolve();
    private lastSaved = 0;
    private readonly recovery = new RecoveryController({
        retry: () => {
            const id = this.store.get().current;
            if (id && this.wantsToPlay)
                void this.load(id, true, { ...this.target, position: this.store.get().position });
        },
        waiting: (attempt, offline) => {
            this.needsReload = true;
            this.phase(
                'buffering',
                offline ? 'Connessione assente.' : `Riconnetto… Tentativo ${attempt} di 3`,
                offline
                    ? 'Riprendo quando torna la rete. Puoi mettere in pausa o scegliere un’altra emittente.'
                    : 'Sto riaprendo la sorgente.',
            );
        },
        exhausted: () =>
            this.phase(
                'error',
                'La sorgente non risponde.',
                'Premi Riprova oppure scegli un’altra emittente.',
            ),
    });

    constructor(private readonly deps: Dependencies) {}

    attach(video: HTMLVideoElement) {
        this.video = video;
        const events = new AbortController();
        video.addEventListener(
            'playing',
            () => {
                if (this.pending || !this.session) return;
                this.recovery.playing();
                this.store.set({
                    phase: 'playing',
                    hasPicture: !isRadio(this.store.get().channel),
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
                if (this.pending) return;
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
            'waiting',
            () => {
                if (!this.pending && this.wantsToPlay) {
                    this.recovery.buffering();
                    this.phase('buffering', 'Riprendiamo il filo.', 'Sto recuperando la sorgente…');
                }
            },
            { signal: events.signal },
        );
        video.addEventListener(
            'error',
            () => {
                this.updatePosition();
                this.saveWatch();
                if (!this.pending && this.session?.stream.continuous && this.wantsToPlay)
                    this.recovery.failed(video.error?.code !== 3 && video.error?.code !== 4);
            },
            { signal: events.signal },
        );
        video.addEventListener(
            'ended',
            () => {
                if (this.pending) return;
                if (this.store.get().mode !== 'live') {
                    this.updatePosition();
                    this.saveWatch(true);
                    this.wantsToPlay = false;
                    this.recovery.stop();
                    this.store.set({ ended: true, paused: true, phase: 'paused' });
                } else if (this.wantsToPlay) this.recovery.failed(true);
            },
            { signal: events.signal },
        );
        const online = () => this.networkChanged(true);
        const offline = () => this.networkChanged(false);
        window.addEventListener('online', online);
        window.addEventListener('offline', offline);
        const leaving = () => {
            this.updatePosition();
            this.saveWatch(false, true);
        };
        window.addEventListener('pagehide', leaving);
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
        video.addEventListener('timeupdate', () => this.updatePosition(), {
            signal: events.signal,
        });
        this.applyPreferences();
        return () => {
            events.abort();
            this.recovery.stop();
            window.removeEventListener('online', online);
            window.removeEventListener('offline', offline);
            window.removeEventListener('pagehide', leaving);
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
                preferredAudioLanguage:
                    this.deps.preferences().audio_language || navigator.language || 'it',
                streaming: { bufferingGoal: 15, rebufferingGoal: 2 },
                manifest: { retryParameters: { maxAttempts: 3, baseDelay: 800, backoffFactor: 2 } },
            });
            player.addEventListener('error', (event) => {
                this.updatePosition();
                this.saveWatch();
                if (this.session?.stream.continuous) return;
                const error = event.detail as { code?: number } | undefined;
                if (this.pending || [7000, 7001].includes(error?.code || 0)) return;
                this.emit({ kind: 'error', ...window.LumePlayback.sanitizedError(error) });
                this.recovery.failed(!!error?.code && error.code >= 1000 && error.code < 2000);
            });
            player.addEventListener('buffering', (event) => {
                if (event.buffering && this.store.get().phase === 'playing' && !this.pending) {
                    this.recovery.buffering();
                    this.phase(
                        'buffering',
                        'Riprendiamo il filo.',
                        'Stiamo recuperando la diretta…',
                    );
                } else if (
                    !event.buffering &&
                    this.store.get().phase === 'buffering' &&
                    !this.pending &&
                    !this.needsReload
                ) {
                    this.recovery.playing();
                    this.phase('playing');
                }
            });
            player.addEventListener('metadata', (event) => {
                if (!isRadio(this.store.get().channel)) return;
                const data = event as unknown as {
                    payload?:
                        | { key?: string; id?: string; data?: unknown }
                        | { key?: string; id?: string; data?: unknown }[];
                };
                const frame = Array.isArray(data.payload)
                    ? data.payload.find((f) => (f.key || f.id) === 'TIT2')
                    : data.payload;
                if (frame && (frame.key || frame.id) === 'TIT2' && typeof frame.data === 'string')
                    this.store.set({
                        nowPlaying: frame.data.replace(/[\u0000-\u001f\u007f]/g, '').slice(0, 300),
                    });
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

    private appliedTrackPreferences = '';
    applyPreferences(forceTracks = false) {
        const preferences = this.deps.preferences();
        if (this.video) {
            this.video.volume = preferences.volume;
            this.video.muted = preferences.muted;
        }
        this.store.set({ volume: preferences.volume, muted: preferences.muted });
        const language = preferences.audio_language || navigator.language || 'it';
        const subtitleLanguage = preferences.subtitle_language || navigator.language || 'it';
        this.player?.configure({
            preferredAudioLanguage: language,
            preferredTextLanguage: preferences.subtitle_mode === 'language' ? subtitleLanguage : '',
            abr: { enabled: true },
            restrictions: {
                maxHeight: preferences.quality === 'auto' ? Infinity : Number(preferences.quality),
            },
        });
        this.applyTrackPreferences(forceTracks);
    }
    private applyTrackPreferences(force: boolean) {
        const p = this.deps.preferences();
        const key = JSON.stringify([p.audio_language, p.subtitle_mode, p.subtitle_language]);
        if (!this.player || this.pending || (!force && key === this.appliedTrackPreferences))
            return;
        this.appliedTrackPreferences = key;
        this.player.selectAudioLanguage(p.audio_language || navigator.language || 'it');
        if (p.subtitle_mode === 'off') this.player.setTextTrackVisibility(false);
        else if (p.subtitle_mode === 'language') {
            const preferred = (p.subtitle_language || navigator.language || 'it').toLowerCase();
            const track =
                this.player.getTextTracks().find((t) => t.language.toLowerCase() === preferred) ||
                this.player
                    .getTextTracks()
                    .find(
                        (t) => t.language.split('-')[0].toLowerCase() === preferred.split('-')[0],
                    );
            if (track) this.player.selectTextTrack(track);
            this.player.setTextTrackVisibility(!!track);
        }
    }
    private updateTracks() {
        if (this.session?.stream.continuous) {
            this.store.set({ height: 0, audioTracks: [], textTracks: [] });
            return;
        }
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

    async load(
        id: string,
        automatic = false,
        target?: { mode?: string; event?: string; position?: number },
    ) {
        const lookup = ++this.lookupGeneration;
        let channel: Channel | undefined;
        try {
            channel = await this.deps.channel(id);
        } catch {
            channel = undefined;
        }
        if (lookup !== this.lookupGeneration) return;
        if (!channel || !this.video) return;
        if (
            !automatic &&
            !target &&
            id === this.store.get().current &&
            ['playing', 'paused', 'buffering'].includes(this.store.get().phase)
        ) {
            this.deps.watch();
            return;
        }
        this.saveWatch();
        this.target = target || (automatic ? this.target : {});
        if (!automatic) {
            this.refreshedSource = '';
            this.recovery.start();
            this.wantsToPlay = true;
        }
        this.needsReload = false;
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
            nowPlaying: '',
            paused: !this.wantsToPlay,
        });
        this.video.pause();
        this.deps.watch();
        this.phase(
            'loading',
            channel.content === 'vod' || this.target.mode === 'archive'
                ? `Caricamento di ${channel.name}…`
                : `Sintonizzazione su ${channel.name}…`,
            channel.content === 'vod' || this.target.mode === 'archive'
                ? 'Preparazione del video.'
                : 'Ancora un istante, il tuo canale sta arrivando.',
        );
        try {
            const signal = this.loadController.signal;
            const session = await bounded(
                api<Session>('/api/play', { channel_id: id, ...this.target }, { signal }),
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
            this.target = {
                mode: session.stream.mode || 'live',
                event: session.stream.event || '',
            };
            this.store.set({
                mode: session.stream.mode || 'live',
                contentTitle: session.stream.title || channel.name,
                position: session.stream.position || 0,
                duration: session.stream.duration || 0,
                seekable: session.stream.seekable !== false,
                ended: false,
            });
            this.video.removeAttribute('src');
            this.video.load();
            let capabilities = await bounded(
                window.LumePlayback.probeCapabilities(this.video),
                12_000,
                signal,
            );
            if (generation !== this.generation) return;
            this.emit({ kind: 'capabilities', ...capabilities });
            if (
                !isRadio(channel) &&
                (!capabilities.avc_mse_supported || !capabilities.aac_mse_supported)
            )
                throw { code: 'codec_unavailable' };
            if (
                session.stream.drm &&
                session.stream.drm.key_system !== 'org.w3.clearkey' &&
                !capabilities.widevine_available
            ) {
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
            // Shaka merges DRM maps. Empty objects keep previous keys and
            // license systems; undefined restores each map to its default.
            player.configure({ drm: { clearKeys: undefined, servers: undefined } });
            player.configure({
                drm: {
                    // Explicit playlist keys override the DRM advertised by DASH.
                    clearKeys: session.stream.drm?.clear_keys || {},
                    servers: session.stream.drm
                        ? {
                              [session.stream.drm.key_system || 'com.widevine.alpha']:
                                  'lume-license:license',
                          }
                        : {},
                },
            });
            this.applyPreferences();
            if (session.stream.continuous) {
                this.video.addEventListener(
                    'loadedmetadata',
                    () => {
                        if (
                            generation === this.generation &&
                            this.video &&
                            session.stream.mode !== 'live'
                        ) {
                            this.video.currentTime = session.stream.position || 0;
                            this.updatePosition();
                        }
                    },
                    { once: true, signal },
                );
                this.video.src = session.stream.url;
            } else
                await bounded(
                    player.load(
                        session.stream.url,
                        session.stream.mode && session.stream.mode !== 'live'
                            ? session.stream.position || 0
                            : undefined,
                        session.stream.mime_type || undefined,
                    ),
                    45_000,
                    signal,
                );
            if (generation !== this.generation) return;
            this.emit({ kind: 'loaded', drm_configured: !!session.stream.drm });
            this.pending = false;
            this.applyPreferences(true);
            if (this.wantsToPlay) await this.resume(false);
            else this.phase('paused');
            if (generation !== this.generation) return;
            this.updateTracks();
            this.sampleTimer = setInterval(() => {
                this.updatePosition();
                if (Date.now() - this.lastSaved >= 5000) {
                    this.saveWatch();
                    this.lastSaved = Date.now();
                }
                if (this.session && this.video && this.video.readyState >= 2)
                    this.emit(this.sample());
                if (isRadio(channel)) void this.pollRadio(generation, session.playback_id);
            }, 2000);
        } catch (error) {
            const failure = error as { name?: string; code?: string };
            if (generation !== this.generation || failure.name === 'AbortError') return;
            if (String(failure.code).startsWith('widevine') || failure.code === 'codec_unavailable')
                this.recovery.stop();
            ++this.generation;
            this.loadController?.abort();
            this.pending = false;
            void this.player?.unload().catch(() => {});
            this.emit(
                { kind: 'error', ...window.LumePlayback.sanitizedError(error) },
                pendingSession,
            );
            if (await this.refreshExpiredSource(error, channel, this.generation, lookup)) return;
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
            else this.recovery.failed(true);
        }
    }
    private async refreshExpiredSource(
        error: unknown,
        channel: Channel,
        generation: number,
        lookup: number,
    ) {
        const failure = error as { code?: number; data?: unknown[] };
        if (
            failure.code !== 1001 ||
            ![401, 403, 404, 410].includes(Number(failure.data?.[0])) ||
            !channel.source_id ||
            this.refreshedSource === channel.source_id
        )
            return false;
        this.refreshedSource = channel.source_id;
        const controller = new AbortController();
        this.loadController = controller;
        const signal = controller.signal;
        const timeout = setTimeout(() => controller.abort(), 60_000);
        try {
            const source = (
                await api<{ sources: PersonalSource[] }>('/api/library', undefined, { signal })
            ).sources.find((s) => s.id === channel.source_id);
            if (generation !== this.generation || lookup !== this.lookupGeneration) return true;
            if (!source?.enabled || source.file || Date.now() - source.checked_at < 30_000)
                return false;
            this.recovery.start();
            this.phase(
                'loading',
                'Aggiorno la lista…',
                'L’indirizzo del canale potrebbe essere cambiato.',
            );
            const deadline = Date.now() + 60_000;
            const job = await api<ImportJob>(
                '/api/library/refresh',
                { id: channel.source_id },
                { signal },
            );
            while (Date.now() < deadline) {
                if (generation !== this.generation || lookup !== this.lookupGeneration) return true;
                if (signal.aborted) return false;
                if (job.id) {
                    const status = await api<ImportJob>(
                        '/api/library/job',
                        { id: job.id },
                        { signal },
                    );
                    if (['error', 'cancelled'].includes(status.phase)) return false;
                    if (status.phase === 'done') break;
                } else {
                    // An existing background refresh owns the job. Wait for its
                    // publication without starting another import or cancelling it.
                    const next = (
                        await api<{ sources: PersonalSource[] }>('/api/library', undefined, {
                            signal,
                        })
                    ).sources.find((s) => s.id === source.id);
                    if (!next?.enabled || next.error) return false;
                    if (next.revision !== source.revision || next.checked_at > source.checked_at)
                        break;
                }
                await new Promise((resolve) => setTimeout(resolve, 250));
            }
            if (generation !== this.generation || lookup !== this.lookupGeneration) return true;
            if (signal.aborted || Date.now() >= deadline) return false;
            await this.load(channel.id, true, {
                ...this.target,
                position: this.store.get().position,
            });
            return true;
        } catch {
            return generation !== this.generation || lookup !== this.lookupGeneration;
        } finally {
            clearTimeout(timeout);
        }
    }
    private async pollRadio(generation: number, playbackId: string) {
        try {
            const status = await api<{ playback_id?: string; audio?: { now_playing?: string } }>(
                '/api/radio/status',
            );
            if (
                generation === this.generation &&
                status.playback_id === playbackId &&
                typeof status.audio?.now_playing === 'string'
            )
                this.store.set({ nowPlaying: status.audio.now_playing });
        } catch {
            /* A metadata failure does not stop the radio. */
        }
    }
    networkChanged(online: boolean) {
        this.recovery.networkChanged(online);
    }
    async resume(manual = true) {
        if (this.store.get().ended) {
            this.restart();
            return;
        }
        const generation = this.generation;
        if (manual) {
            const reload =
                this.needsReload ||
                this.store.get().phase === 'error' ||
                (this.store.get().mode === 'live' &&
                    !!this.session?.stream.continuous &&
                    this.store.get().phase === 'paused');
            this.wantsToPlay = true;
            this.recovery.start();
            if (reload) {
                const id = this.store.get().current;
                if (id) {
                    await this.load(id, true, {
                        ...this.target,
                        position: this.store.get().position,
                    });
                    return;
                }
            }
        }
        try {
            await this.video?.play();
        } catch (error) {
            const name = (error as { name?: string }).name;
            if (generation !== this.generation || name === 'AbortError') return;
            if (name !== 'NotAllowedError') {
                this.recovery.stop();
                this.wantsToPlay = false;
                this.store.set({ paused: true });
                this.emit({ kind: 'error', ...window.LumePlayback.sanitizedError(error) });
                this.phase(
                    'error',
                    'Non riesco ad avviare il video.',
                    'Riprova o scegli un altro contenuto.',
                );
                return;
            }
            this.store.set({ paused: true });
            this.phase(
                'interaction',
                this.store.get().mode === 'live'
                    ? 'Il tuo canale è pronto.'
                    : 'Il tuo video è pronto.',
                'Premi Avvia per iniziare la visione con audio.',
            );
        }
    }
    togglePause() {
        if (!this.wantsToPlay || this.store.get().phase === 'paused') void this.resume();
        else {
            this.wantsToPlay = false;
            this.recovery.stop();
            this.video?.pause();
            this.saveWatch();
            this.store.set({ paused: true, phase: 'paused' });
        }
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
        void this.deps.changedPreferences?.({ audio_language: language }).catch(() => {});
    }
    text(id: string) {
        if (!this.player) return;
        const track = this.player.getTextTracks().find((t) => String(t.id) === id);
        if (track) this.player.selectTextTrack(track);
        this.player.setTextTrackVisibility(!!track);
        this.store.set({ text: id });
        void this.deps
            .changedPreferences?.(
                track
                    ? { subtitle_mode: 'language', subtitle_language: track.language }
                    : { subtitle_mode: 'off' },
            )
            .catch(() => {});
    }
    goLive() {
        if (this.store.get().mode === 'archive') {
            const id = this.store.get().current;
            if (id) void this.load(id, false, { mode: 'live' });
            return;
        }
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
        ++this.lookupGeneration;
        this.saveWatch();
        this.wantsToPlay = false;
        this.recovery.stop();
        const generation = ++this.generation;
        const playbackId = this.session?.playback_id;
        this.loadController?.abort();
        this.pending = false;
        clearInterval(this.sampleTimer);
        void api('/api/stop', { playback_id: playbackId || null }).catch(() => {});
        this.video?.pause();
        await this.player?.unload().catch(() => {});
        this.video?.removeAttribute('src');
        this.video?.load();
        if (generation !== this.generation) return;
        this.session = null;
        this.store.set({
            ...initial,
            volume: this.video?.volume || 0,
            muted: this.video?.muted || false,
        });
        this.deps.stopped();
    }
    private updatePosition() {
        if (!this.video || !this.session || this.pending || this.store.get().mode === 'live')
            return;
        const base = this.session.stream.base_offset || 0;
        const duration =
            this.store.get().mode === 'archive'
                ? this.session.stream.duration || 0
                : Number.isFinite(this.video.duration)
                  ? this.video.duration
                  : this.session.stream.duration || 0;
        this.store.set({ position: Math.max(0, base + this.video.currentTime), duration });
    }
    private saveWatch(completed = false, leaving = false) {
        const state = this.store.get();
        if (!state.channel?.source_id || state.mode === 'live' || !this.session || this.pending)
            return;
        const body = {
            id: state.current,
            mode: state.mode,
            event: this.session.stream.event || '',
            revision: this.session.stream.watch_revision,
            position: state.position,
            duration: state.duration,
            completed: completed || state.ended,
        };
        if (leaving) {
            void api('/api/library/watch', body, { keepalive: true }).catch(() => {});
            return;
        }
        this.watchQueue = this.watchQueue
            .catch(() => {})
            .then(async () => {
                await api('/api/library/watch', body);
            })
            .catch(() => {});
    }
    seek(position: number) {
        const state = this.store.get();
        if (!state.seekable || state.mode === 'live' || !this.video || !state.current) return;
        position = Math.max(
            0,
            Math.min(position, state.duration ? Math.max(0, state.duration - 0.1) : position),
        );
        if (state.mode === 'archive')
            void this.load(state.current, true, { ...this.target, position });
        else {
            this.store.set({ ended: false });
            this.video.currentTime = position;
            this.updatePosition();
            this.saveWatch();
        }
    }
    seekBy(seconds: number) {
        this.seek(this.store.get().position + seconds);
    }
    restart() {
        const id = this.store.get().current;
        if (id) {
            this.wantsToPlay = true;
            this.recovery.start();
            void this.load(id, true, { ...this.target, position: 0 });
        }
    }
}
