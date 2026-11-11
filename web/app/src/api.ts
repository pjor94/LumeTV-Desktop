let token = '';

export function initializeSession() {
    const fragment = decodeURIComponent(location.hash.slice(1));
    token = /^[A-Za-z0-9_-]{16,256}$/.test(fragment)
        ? fragment
        : sessionStorage.getItem('lume-session') || '';
    if (token) sessionStorage.setItem('lume-session', token);
    history.replaceState(null, '', location.pathname + location.search);
    if (!token) throw new Error('session_missing');
}
export function apiFetch(path: string, options: RequestInit = {}) {
    const headers = new Headers(options.headers);
    headers.set('X-Lume-Token', token);
    return fetch(path, { ...options, cache: 'no-store', credentials: 'omit', headers });
}
export async function api<T>(path: string, body?: unknown, options: RequestInit = {}): Promise<T> {
    const headers = new Headers(options.headers);
    if (body !== undefined) headers.set('Content-Type', 'application/json');
    const response = await apiFetch(path, {
        ...options,
        headers,
        ...(body === undefined ? {} : { method: 'POST', body: JSON.stringify(body) }),
    });
    if (!response.ok) {
        const error = new Error('Local request failed') as Error & {
            status: number;
            code?: string;
        };
        error.status = response.status;
        try {
            error.code = (await response.json()).error;
        } catch {
            /* HTTP status remains available. */
        }
        throw error;
    }
    return response.json() as Promise<T>;
}
export async function bounded<T>(
    promise: Promise<T>,
    milliseconds: number,
    signal?: AbortSignal,
): Promise<T> {
    let timeout: ReturnType<typeof setTimeout> | undefined;
    let abort: (() => void) | undefined;
    try {
        return await Promise.race([
            promise,
            new Promise<never>((_, reject) => {
                timeout = setTimeout(() => reject({ code: 'startup_timeout' }), milliseconds);
                abort = () => reject(new DOMException('Cancelled', 'AbortError'));
                if (signal?.aborted) abort();
                else signal?.addEventListener('abort', abort, { once: true });
            }),
        ]);
    } finally {
        clearTimeout(timeout);
        if (abort) signal?.removeEventListener('abort', abort);
    }
}

// Both playback history and user edits use this queue, so a late /api/played
// response cannot overwrite a more recent favorite, volume or theme change.
export function createPreferenceQueue<T>(
    request: (path: string, body: unknown) => Promise<T>,
    apply: (value: T) => void,
) {
    let queue = Promise.resolve();
    return (patch: unknown, path = '/api/preferences') => {
        const next = queue
            .catch(() => {})
            .then(async () => {
                const value = await request(path, patch);
                apply(value);
                return value;
            });
        queue = next.then(
            () => {},
            () => {},
        );
        return next;
    };
}
