export interface Channel {
    id: string;
    name: string;
    number: number;
    provider: string;
    group: string;
    color: string;
}
export interface Programme {
    title: string;
    description?: string;
    start: number;
    end: number;
}
export interface Reminder {
    channel_id: string;
    title: string;
    start: number;
}
export type Quality = 'auto' | '1080' | '720' | '480';
export interface Preferences {
    favorites: string[];
    recent: string[];
    reminders: Reminder[];
    theme: 'dark' | 'light' | 'system';
    volume: number;
    muted: boolean;
    quality: Quality;
    zap_favorites_only: boolean;
}
export interface CatalogStatus {
    updated_at: number;
    loading: boolean;
    error?: string | null;
    revision: string;
    next_retry_at: number;
}
export interface Catalog extends CatalogStatus {
    channels: Channel[];
}
export interface Guide {
    programmes: Record<string, Programme[]>;
    logos: Record<string, string>;
    updated_at: number;
    loading: boolean;
    error?: string | null;
    next_retry_at?: number;
}
export interface Bootstrap {
    channels: Channel[];
    preferences: Preferences;
    catalog: CatalogStatus;
    version: string;
    os: string;
}
export interface Session {
    playback_id: string;
    channel: Channel;
    stream: { url: string; drm?: unknown; mime_type?: string };
}
export type Route = 'channels' | 'discover' | 'guide' | 'favorites' | 'settings' | 'watch';
export type Period = 'now' | 'soon' | 'tonight';
export type WidevinePhase =
    'unavailable' | 'checking' | 'downloading' | 'installing' | 'installed' | 'error';
export interface WidevineStatus {
    phase: WidevinePhase;
    version: string;
    installed_at_startup: boolean;
    restart_required: boolean;
    error: string | null;
    elapsed_seconds: number;
}
export type PlaybackPhase =
    'idle' | 'loading' | 'playing' | 'paused' | 'buffering' | 'error' | 'interaction' | 'restart';
export interface Track {
    id: number;
    language: string;
    label?: string;
    height?: number;
    active: boolean;
}
export interface PlaybackState {
    current: string | null;
    channel: Channel | null;
    phase: PlaybackPhase;
    title: string;
    detail: string;
    hasPicture: boolean;
    paused: boolean;
    muted: boolean;
    volume: number;
    height: number;
    audioTracks: Track[];
    textTracks: Track[];
    audio: string;
    text: string;
    widevine: WidevineStatus | null;
    setupElapsed: number;
}
export type Dialog =
    { kind: 'programme'; key: string } | { kind: 'reminders' | 'shortcuts' } | null;
export interface Toast {
    id: number;
    message: string;
    action?: { label: string; run: () => void };
    dismissing?: boolean;
}
export interface AppState {
    ready: boolean;
    failed: boolean;
    channels: Channel[];
    preferences: Preferences;
    catalog: CatalogStatus;
    guide: Guide;
    page: Route;
    previousPage: Route;
    query: string;
    provider: string;
    group: string;
    period: Period;
    favoritesOnly: boolean;
    discoveryKey: string | null;
    guideStart: number;
    watchFavorites: boolean;
    version: string;
    os: string;
    now: number;
    dialog: Dialog;
    dialogOpen: boolean;
    toasts: Toast[];
    scrollRevision: number;
}
