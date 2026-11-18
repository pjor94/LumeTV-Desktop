import { apiFetch } from './api';
import { createStore } from './store';
import { saveVideoAction } from './app';
export interface VideoItem {
    id: string;
    source: string;
    title: string;
    kind: 'movie' | 'episode' | 'video';
    series: string;
    series_title: string;
    season: number | null;
    episode: number | null;
    year: number | null;
    duration: number;
    description: string;
    genres: string[];
    groups: string[];
    added: number;
    episodes: number;
    position: number;
    watched_duration: number;
    completed: boolean;
    hidden: boolean;
    saved: boolean;
    poster: boolean;
    backdrop: boolean;
    cast: string[];
    directors: string[];
    overview_language: string;
    online: boolean;
    metadata_type?: string;
}
export interface VideoQuery {
    source?: string;
    group?: string;
    query?: string;
    kind?: string;
    shelf?: string;
    order?: string;
    series?: string;
    season?: number | null;
    episodes?: boolean;
    offset?: number;
    limit?: number;
}
export interface VideoDetail {
    item: VideoItem;
    target: string;
    target_item: VideoItem;
    next: string | null;
    seasons: { season: number; episodes: number }[];
}
export interface VideoShelf {
    id: string;
    title: string;
    kind: string;
    shelf: string;
    rows: VideoItem[];
    total: number;
}
export interface MetadataSettings {
    enabled: boolean;
    configured: boolean;
}
export interface Candidate {
    id: number;
    kind: string;
    title: string;
    original_title: string;
    year: number | null;
}
export interface MetadataResult {
    status: string;
    candidates: Candidate[];
}
export const videoChanges = createStore({ revision: 0 });
const artworkCache = new Map<string, Blob>();
const artworkFlights = new Map<
    string,
    { promise: Promise<Blob>; controller: AbortController; readers: number }
>();
let artworkBytes = 0;
export function artworkDataUrl(blob: Blob, signal: AbortSignal) {
    return new Promise<string>((resolve, reject) => {
        const reader = new FileReader();
        const abort = () => {
            reader.abort();
            reject(new DOMException('Cancelled', 'AbortError'));
        };
        const done = () => signal.removeEventListener('abort', abort);
        reader.onload = () => {
            done();
            resolve(String(reader.result));
        };
        reader.onerror = () => {
            done();
            reject(new Error('artwork_unavailable'));
        };
        reader.onabort = () => {
            done();
            reject(new DOMException('Cancelled', 'AbortError'));
        };
        if (signal.aborted) {
            abort();
            return;
        }
        signal.addEventListener('abort', abort, { once: true });
        reader.readAsDataURL(blob);
    });
}
export async function videoArtwork(key: string, path: string, signal: AbortSignal) {
    const cached = artworkCache.get(key);
    if (cached) {
        artworkCache.delete(key);
        artworkCache.set(key, cached);
        return cached;
    }
    let flight = artworkFlights.get(key);
    if (!flight) {
        const controller = new AbortController();
        const promise = apiFetch(path, { signal: controller.signal })
            .then(async (response) => {
                if (!response.ok) throw new Error('artwork_unavailable');
                const blob = await response.blob();
                if (blob.size > 2 * 1024 * 1024 || controller.signal.aborted)
                    throw new Error('artwork_unavailable');
                artworkCache.set(key, blob);
                artworkBytes += blob.size;
                for (const [oldKey, old] of artworkCache) {
                    if (artworkBytes <= 16 * 1024 * 1024 && artworkCache.size <= 100) break;
                    artworkCache.delete(oldKey);
                    artworkBytes -= old.size;
                }
                return blob;
            })
            .finally(() => {
                if (artworkFlights.get(key)?.controller === controller) artworkFlights.delete(key);
            });
        flight = { promise, controller, readers: 0 };
        artworkFlights.set(key, flight);
    }
    flight.readers++;
    const current = flight;
    let aborted: (() => void) | undefined;
    try {
        return await Promise.race([
            current.promise,
            new Promise<never>((_, reject) => {
                aborted = () => reject(new DOMException('Cancelled', 'AbortError'));
                if (signal.aborted) aborted();
                else signal.addEventListener('abort', aborted, { once: true });
            }),
        ]);
    } finally {
        if (aborted) signal.removeEventListener('abort', aborted);
        current.readers--;
        if (!current.readers && artworkFlights.get(key) === current) {
            current.controller.abort();
            artworkFlights.delete(key);
        }
    }
}
export function changedVideos() {
    videoChanges.set({ revision: videoChanges.get().revision + 1 });
}
export async function videoAction(
    item: VideoItem,
    action: 'saved' | 'completed' | 'hidden',
    enabled: boolean,
) {
    await saveVideoAction({ id: item.id, action, enabled });
    changedVideos();
}
export function videoTime(seconds: number) {
    seconds = Math.floor(Math.max(0, seconds));
    const minutes = Math.floor(seconds / 60);
    return `${minutes}:${String(seconds % 60).padStart(2, '0')}`;
}
export function videoInfo(item: VideoItem, series = false) {
    return [
        item.year,
        series
            ? `${item.episodes} episodi`
            : item.season !== null && item.episode !== null
              ? `S${item.season} · E${item.episode}`
              : item.kind === 'movie'
                ? 'Film'
                : 'Video',
        !series && item.duration > 0 ? `${Math.round(item.duration / 60)} min` : '',
    ]
        .filter(Boolean)
        .join(' · ');
}
