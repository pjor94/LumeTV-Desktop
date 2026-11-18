import { api } from './api';
import { createStore } from './store';
import type { Channel, Programme } from './types';

export type Resource = 'playlist' | 'manifest' | 'media' | 'license' | 'key' | 'logo' | 'guide';
export interface Access {
    headers: Record<string, string>;
    origins: string[];
}
export interface GuideSettings {
    enabled: boolean;
    urls: string[];
    allow_http: boolean;
    language: string;
    timezone: string;
    shift_minutes: number;
    history_days: number;
    future_hours: number;
    mappings: Record<string, string>;
    archive_days: number | null;
    archive_timezone: string;
    ongoing: boolean;
    before_seconds: number;
    after_seconds: number;
}
export interface PlaylistSettings {
    kind: 'tv' | 'radio';
    allow_http: boolean;
    allow_http_logos: boolean;
    cookies: boolean;
    encoding: string;
    format: string;
    group_profile: string;
    content: string;
    vod_detection: string;
    group_content: Record<string, string>;
    refresh_minutes: number;
    access: Partial<Record<Resource, Access>>;
    guide: GuideSettings;
}
export interface ImportReport {
    count: number;
    scanned: number;
    excluded: number;
    duplicates: number;
    vod: number;
    radio: number;
    bytes: number;
    sample: string[];
    diagnostics: Record<string, number>;
    guide_urls: string[];
    refresh_minutes: number;
}
export interface GuideStatus {
    origin?: 'lume' | 'personal' | 'none';
    channel_count?: number;
    supplement?: {
        name: string;
        channel_count: number;
        programmes: number;
        updated_at: number;
        loading: boolean;
        error: string;
        next_retry_at: number;
    };
    updated_at: number;
    loading: boolean;
    matched: number;
    programmes: number;
    error: string;
    next_retry_at: number;
}
export interface PersonalSource {
    id: string;
    name: string;
    enabled: boolean;
    revision: number;
    updated_at: number;
    checked_at: number;
    retry_at: number;
    report: ImportReport;
    guide: GuideStatus;
    error: string;
    file: boolean;
    has_vod: boolean;
}
export interface SourceDetail {
    id: string;
    name: string;
    enabled: boolean;
    revision: number;
    url: string | null;
    settings: PlaylistSettings;
}
export interface ImportJob {
    name?: string;
    id: string;
    source_id: string;
    phase: string;
    bytes: number;
    entries: number;
    report: ImportReport | null;
    error: string;
}
export interface LibraryPage<T> {
    rows: T[];
    total: number;
}
export interface PersonalHome {
    total: number;
    featured: Channel | null;
    recent: Channel[];
}
export interface PersonalProgramme extends Programme {
    event: string;
    archive: boolean;
    ongoing: boolean;
}
export const library = createStore({
    sources: [] as PersonalSource[],
    imports: [] as ImportJob[],
    favoriteCount: 0,
    revision: '',
    ready: false,
    error: '',
});
export const defaultSettings = (): PlaylistSettings => ({
    kind: 'tv',
    allow_http: false,
    allow_http_logos: false,
    cookies: false,
    encoding: 'utf-8',
    format: 'extended',
    group_profile: 'default',
    content: 'auto',
    vod_detection: 'prudent',
    group_content: {},
    refresh_minutes: 0,
    access: {},
    guide: {
        enabled: false,
        urls: [],
        allow_http: false,
        language: 'it',
        timezone: 'Europe/Rome',
        shift_minutes: 0,
        history_days: 7,
        future_hours: 48,
        mappings: {},
        archive_days: null,
        archive_timezone: 'Europe/Rome',
        ongoing: false,
        before_seconds: 0,
        after_seconds: 0,
    },
});
export function errorMessage(error: unknown) {
    const value = error as { code?: string; message?: string };
    return value.code && !/^[a-z_]+$/.test(value.code)
        ? value.code
        : 'Non riesco a completare la richiesta. Riprova.';
}
let pending: Promise<void> | null = null;
export function refreshLibrary() {
    if (pending) return pending;
    pending = api<{
        sources: PersonalSource[];
        imports?: ImportJob[];
        favorite_count?: number;
        layout_revision?: string;
        metadata_revision?: string;
    }>('/api/library')
        .then(
            ({
                sources,
                imports = [],
                favorite_count = 0,
                layout_revision = '',
                metadata_revision = '',
            }) => {
                const revision =
                    sources.map((s) => `${s.id}:${s.revision}:${s.guide.updated_at}`).join('|') +
                    ':' +
                    layout_revision +
                    ':' +
                    metadata_revision;
                library.set({
                    sources,
                    imports,
                    favoriteCount: favorite_count,
                    revision,
                    ready: true,
                    error: '',
                });
            },
        )
        .catch(() =>
            library.set({
                ready: true,
                error: 'Le liste personali non sono raggiungibili. Riprova.',
            }),
        )
        .finally(() => {
            pending = null;
        });
    return pending;
}
export function connectLibrary() {
    void refreshLibrary();
    const timer = setInterval(() => void refreshLibrary(), 30_000);
    return () => clearInterval(timer);
}
export const catalogChannel = (id: string) => api<Channel>('/api/library/channel', { id });
export const personalCatalog = (query: object, signal?: AbortSignal) =>
    api<LibraryPage<Channel>>('/api/library/catalog', query, { signal });
export function readSession<T>(key: string, fallback: T): T {
    try {
        return JSON.parse(sessionStorage.getItem(`lume-${key}`) || 'null') ?? fallback;
    } catch {
        return fallback;
    }
}
export function writeSession(key: string, value: unknown) {
    try {
        sessionStorage.setItem(`lume-${key}`, JSON.stringify(value));
    } catch {
        /* Browsing remains available. */
    }
}
export const librarySelection = createStore(
    readSession('library-selection', { source: 'builtin', group: '', query: '' }),
);
librarySelection.subscribe(() => writeSession('library-selection', librarySelection.get()));
export const personalRadioSelection = createStore(
    readSession('personal-radio-selection', {
        source: 'builtin',
        group: '',
        query: '',
        favorites: false,
    }),
);
personalRadioSelection.subscribe(() =>
    writeSession('personal-radio-selection', personalRadioSelection.get()),
);
export const videoSelection = createStore(
    readSession('video-selection', { source: '', group: '', query: '', favorites: false }),
);
videoSelection.subscribe(() => writeSession('video-selection', videoSelection.get()));

// Owns at most three windows. Obsolete replies never populate a new query.
export class ContinuousData<T> {
    private blocks = new Map<number, T[]>();
    private failed = new Set<number>();
    private stale = new Set<number>();
    private wanted: number[] = [0];
    private generation = 0;
    private request: AbortController | null = null;
    private listeners = new Set<() => void>();
    private revision = 0;
    total = 0;
    loading = true;
    error = '';
    constructor(
        readonly blockSize: number,
        private load: (offset: number, signal: AbortSignal) => Promise<LibraryPage<T>>,
    ) {}
    subscribe = (listener: () => void) => {
        this.listeners.add(listener);
        return () => {
            this.listeners.delete(listener);
        };
    };
    snapshot = () => this.revision;
    private changed() {
        this.revision++;
        for (const listener of this.listeners) listener();
    }
    bind(load: (offset: number, signal: AbortSignal) => Promise<LibraryPage<T>>, anchor = 0) {
        this.close();
        this.load = load;
        this.total = 0;
        this.error = '';
        this.loading = true;
        this.wanted = [Math.floor(anchor / this.blockSize)];
        this.changed();
        void this.pump();
    }
    viewport(position: number) {
        const block = Math.floor(Math.max(0, position) / this.blockSize);
        this.wanted = [block, block + 1, block - 1].filter(
            (b) => b >= 0 && (!this.total || b * this.blockSize < this.total),
        );
        for (const key of this.blocks.keys())
            if (!this.wanted.includes(key)) this.blocks.delete(key);
        for (const key of this.failed) if (!this.wanted.includes(key)) this.failed.delete(key);
        for (const key of this.stale) if (!this.wanted.includes(key)) this.stale.delete(key);
        void this.pump();
    }
    get(position: number) {
        return this.blocks.get(Math.floor(position / this.blockSize))?.[position % this.blockSize];
    }
    get cachedRows() {
        return [...this.blocks.values()].reduce((n, rows) => n + rows.length, 0);
    }
    retry() {
        this.failed.clear();
        this.error = '';
        void this.pump();
    }
    refresh() {
        for (const block of this.wanted) if (this.blocks.has(block)) this.stale.add(block);
        void this.pump();
    }
    private async pump() {
        if (this.request) return;
        const generation = this.generation;
        while (generation === this.generation) {
            const block = this.wanted.find(
                (b) => (!this.blocks.has(b) || this.stale.has(b)) && !this.failed.has(b),
            );
            if (block === undefined) break;
            const request = new AbortController();
            this.request = request;
            this.loading = true;
            this.changed();
            try {
                const page = await this.load(block * this.blockSize, request.signal);
                if (generation !== this.generation) return;
                this.total = Math.max(0, page.total);
                if (block * this.blockSize >= this.total && this.total > 0) {
                    this.wanted = [Math.floor((this.total - 1) / this.blockSize)];
                } else if (this.wanted.includes(block))
                    this.blocks.set(block, page.rows.slice(0, this.blockSize));
                this.error = '';
                this.stale.delete(block);
            } catch (error) {
                if (generation !== this.generation) return;
                this.failed.add(block);
                this.error = errorMessage(error);
            } finally {
                if (generation === this.generation) {
                    this.request = null;
                    this.loading = false;
                    this.changed();
                }
            }
        }
    }
    close() {
        this.generation++;
        this.request?.abort();
        this.request = null;
        this.blocks.clear();
        this.failed.clear();
        this.stale.clear();
    }
}
