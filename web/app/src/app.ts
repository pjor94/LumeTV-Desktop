import { api, bounded, createPreferenceQueue, initializeSession } from './api';
import { createStore } from './store';
import { matchesProvider } from './domain';
import { PlaybackController } from './playback';
import { StartupController } from './startup';
import { motion, type DockRect } from './motion';
import type { AppState, Bootstrap, Catalog, Guide, Preferences, Route } from './types';

export const app = createStore<AppState>({
    ready: false,
    failed: false,
    channels: [],
    preferences: {
        favorites: [],
        recent: [],
        reminders: [],
        theme: 'dark',
        volume: 0.7,
        muted: false,
        quality: 'auto',
        zap_favorites_only: false,
    },
    catalog: { updated_at: 0, loading: false, error: null, revision: '', next_retry_at: 0 },
    guide: { programmes: {}, logos: {}, updated_at: 0, loading: true },
    page: 'channels',
    previousPage: 'channels',
    query: '',
    provider: 'all',
    group: 'all',
    period: 'now',
    favoritesOnly: false,
    discoveryKey: null,
    guideStart: Math.floor(Date.now() / 1_800_000) * 1_800_000,
    watchFavorites: false,
    version: '',
    os: '',
    now: Date.now(),
    dialog: null,
    dialogOpen: false,
    toasts: [],
    scrollRevision: 0,
});
export const startup = new StartupController((signal) => {
    const video = document.createElement('video');
    return bounded(window.LumePlayback.probeCapabilities(video), 12_000, signal);
});
export const playback = new PlaybackController({
    channel: (id) => app.get().channels.find((c) => c.id === id),
    preferences: () => app.get().preferences,
    played: (playback_id) => preferenceRequest({ playback_id }, '/api/played'),
    watch: () => navigate('watch'),
    stopped: () => {
        const state = app.get();
        if (state.page === 'watch')
            navigate(state.previousPage === 'watch' ? 'channels' : state.previousPage);
    },
});
const preferenceRequest = createPreferenceQueue<Preferences>(
    (path, body) => api(path, body),
    (preferences) => {
        app.set({ preferences });
        playback.applyPreferences();
    },
);
export async function savePreferences(patch: unknown, message?: string) {
    try {
        const preferences = await preferenceRequest(patch);
        if (message) toast(message);
        return preferences;
    } catch {
        toast('Non riesco a salvare le modifiche. Riprova.');
        return null;
    }
}
let toastId = 0;
export function toast(message: string, action?: { label: string; run: () => void }) {
    const id = ++toastId;
    app.set({ toasts: [...app.get().toasts.slice(-2), { id, message, action }] });
    setTimeout(() => dismissToast(id), action ? 12_000 : 3500);
}
export function dismissToast(id: number) {
    if (!app.get().toasts.some((t) => t.id === id && !t.dismissing)) return;
    app.set({
        toasts: app.get().toasts.map((t) => (t.id === id ? { ...t, dismissing: true } : t)),
    });
    setTimeout(() => app.set({ toasts: app.get().toasts.filter((t) => t.id !== id) }), 160);
}
let dockBefore: DockRect | null = null;
function captureDock() {
    const dock = document.getElementById('player-dock');
    if (dock) dockBefore = motion.captureDock(dock);
}
export function takeDockTransition() {
    const before = dockBefore;
    dockBefore = null;
    return before;
}
export function setQuery(query: string) {
    captureDock();
    app.set({ query });
}
export function navigate(page: Route) {
    const state = app.get();
    if (page === 'watch' && !playback.store.get().current) return;
    captureDock();
    app.set({
        page,
        previousPage: state.page !== 'watch' ? state.page : state.previousPage,
        query: '',
        dialogOpen: false,
        ...(page !== state.page || !['channels', 'favorites'].includes(page)
            ? { provider: 'all', group: 'all' }
            : {}),
        scrollRevision: state.scrollRevision + 1,
    });
}
export function play(id: string) {
    app.set({ dialogOpen: false });
    void playback.load(id);
}
export function showProgramme(key: string) {
    app.set({ dialog: { kind: 'programme', key }, dialogOpen: true });
}
export function showReminders() {
    app.set({ dialog: { kind: 'reminders' }, dialogOpen: true });
}
export function showShortcuts() {
    app.set({ dialog: { kind: 'shortcuts' }, dialogOpen: true });
}
export function closeDialog() {
    app.set({ dialogOpen: false });
}
export function resetFilters() {
    app.set({ provider: 'all', group: 'all' });
    navigate('channels');
}
export async function toggleReminder(key: string) {
    const [id, start] = key.split('~');
    const state = app.get();
    const programme =
        state.guide.programmes[id]?.find((p) => String(p.start) === start) ||
        state.preferences.reminders.find((r) => r.channel_id === id && String(r.start) === start);
    if (programme)
        await savePreferences(
            { toggle_reminder: { channel_id: id, start: Number(start), title: programme.title } },
            'Promemoria aggiornato.',
        );
}
export function zap(direction: number) {
    const { channels: all, preferences } = app.get();
    const channels =
        preferences.zap_favorites_only && preferences.favorites.length
            ? all.filter((c) => preferences.favorites.includes(c.id))
            : all;
    if (!channels.length) return;
    const index = channels.findIndex((c) => c.id === playback.store.get().current);
    play(channels[(Math.max(0, index) + direction + channels.length) % channels.length].id);
}
let guidePoll: ReturnType<typeof setTimeout> | undefined;
let catalogPoll: ReturnType<typeof setTimeout> | undefined;
let catalogRequest: Promise<void> | null = null;
let guideRequest: Promise<void> | null = null;
export function refreshCatalog(force = false): Promise<void> {
    if (catalogRequest) return catalogRequest;
    clearTimeout(catalogPoll);
    catalogRequest = (async () => {
        try {
            const { channels, ...catalog } = await api<Catalog>(
                force ? '/api/catalog/refresh' : '/api/catalog',
                force ? {} : undefined,
            );
            const state = app.get();
            const changed = catalog.revision !== state.catalog.revision;
            app.set({
                catalog,
                ...(changed
                    ? {
                          channels,
                          provider: channels.some((c) => matchesProvider(c, state.provider))
                              ? state.provider
                              : 'all',
                          group:
                              state.group === 'all' || channels.some((c) => c.group === state.group)
                                  ? state.group
                                  : 'all',
                      }
                    : {}),
            });
            if (changed) await refreshGuide();
            catalogPoll = setTimeout(() => void refreshCatalog(), catalog.loading ? 1800 : 300_000);
            if (force && !catalog.loading) toast(catalog.error || 'Il catalogo è aggiornato.');
        } catch {
            app.set({
                catalog: {
                    ...app.get().catalog,
                    loading: false,
                    error: 'Non riesco ad aggiornare la lista. I canali salvati restano disponibili.',
                },
            });
            catalogPoll = setTimeout(() => void refreshCatalog(), 300_000);
        }
    })().finally(() => {
        catalogRequest = null;
    });
    return catalogRequest;
}
export function refreshGuide(force = false): Promise<void> {
    if (guideRequest) return guideRequest;
    clearTimeout(guidePoll);
    guideRequest = (async () => {
        try {
            const guide = await api<Guide>(
                force ? '/api/guide/refresh' : '/api/guide',
                force ? {} : undefined,
            );
            app.set({ guide });
            guidePoll = setTimeout(() => void refreshGuide(), guide.loading ? 1800 : 300_000);
            if (force && !guide.loading) toast(guide.error || 'La guida è aggiornata.');
        } catch {
            app.set({
                guide: {
                    ...app.get().guide,
                    loading: false,
                    error: 'Non riesco ad aggiornare la guida. I canali restano disponibili.',
                },
            });
            guidePoll = setTimeout(() => void refreshGuide(), 300_000);
        }
    })().finally(() => {
        guideRequest = null;
    });
    return guideRequest;
}
let bootstrapPromise: Promise<void> | null = null;
const reminderShown = new Set<string>();
function tick() {
    const now = Date.now();
    app.set({ now });
    for (const reminder of app.get().preferences.reminders) {
        const key = `${reminder.channel_id}~${reminder.start}`;
        if (
            !reminderShown.has(key) &&
            reminder.start - now <= 60_000 &&
            reminder.start >= now - 120_000
        ) {
            reminderShown.add(key);
            toast(`Sta per iniziare: ${reminder.title}`, {
                label: 'Guarda',
                run: () => play(reminder.channel_id),
            });
        }
    }
}
export function bootstrap() {
    if (bootstrapPromise) return bootstrapPromise;
    bootstrapPromise = (async () => {
        try {
            initializeSession();
            const initial = await api<Bootstrap>('/api/bootstrap');
            app.set({
                channels: initial.channels,
                preferences: initial.preferences,
                catalog: initial.catalog,
                version: initial.version,
                os: initial.os,
                ready: true,
            });
            playback.applyPreferences();
            void startup.start();
            tick();
            await Promise.all([refreshGuide(), refreshCatalog()]);
            setInterval(tick, 15_000);
            window.addEventListener('online', () => {
                if (startup.store.get().phase === 'error') void startup.start();
                const state = playback.store.get();
                if (state.phase === 'error' && state.current) play(state.current);
                if (app.get().guide.error) void refreshGuide();
                void refreshCatalog();
            });
        } catch {
            app.set({ failed: true });
        }
    })();
    return bootstrapPromise;
}
