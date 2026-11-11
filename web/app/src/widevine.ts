import { api, bounded } from './api';
import type { Capabilities } from './runtime';
import type { WidevineStatus } from './types';

export class WidevineSetupError extends Error {
    constructor(
        readonly code: 'widevine_unavailable' | 'widevine_restart' | 'widevine_service_error',
    ) {
        super(code);
    }
}
interface SetupOptions {
    signal: AbortSignal;
    probe: () => Promise<Capabilities>;
    update: (status: WidevineStatus | null, elapsed: number) => void;
    request?: typeof api;
    timeout?: number;
    now?: () => number;
    sleep?: (ms: number, signal: AbortSignal) => Promise<void>;
}
function sleep(ms: number, signal: AbortSignal) {
    return new Promise<void>((resolve, reject) => {
        const abort = () => {
            clearTimeout(timer);
            reject(new DOMException('Cancelled', 'AbortError'));
        };
        const timer = setTimeout(() => {
            signal.removeEventListener('abort', abort);
            resolve();
        }, ms);
        if (signal.aborted) abort();
        else signal.addEventListener('abort', abort, { once: true });
    });
}

// Installed and usable are different: EME is always the final readiness check.
export async function prepareWidevine(
    initial: Capabilities,
    options: SetupOptions,
): Promise<Capabilities> {
    if (initial.widevine_available) return initial;
    const {
        signal,
        probe,
        update,
        request = api,
        timeout = 300_000,
        now = () => performance.now(),
        sleep: wait = sleep,
    } = options;
    if (
        !initial.secure_context ||
        !initial.eme_available ||
        !initial.avc_mse_supported ||
        !initial.aac_mse_supported
    )
        throw new WidevineSetupError('widevine_unavailable');
    const started = now();
    const elapsed = () => Math.max(0, now() - started) / 1000;
    let native = true;
    let status: WidevineStatus | null = null;
    try {
        status = await bounded(
            request<WidevineStatus>('/api/widevine/update', {}, { signal }),
            5000,
            signal,
        );
    } catch (error) {
        if (signal.aborted) throw error;
        native = false;
    }
    update(status, elapsed());
    while (!signal.aborted && now() - started < timeout) {
        if (native) {
            try {
                status = await bounded(
                    request<WidevineStatus>('/api/widevine', undefined, { signal }),
                    5000,
                    signal,
                );
            } catch (error) {
                if (signal.aborted) throw error;
                status = null;
            }
        }
        update(status, elapsed());
        const capabilities = await bounded(probe(), 12_000, signal);
        if (capabilities.widevine_available) return capabilities;
        if (status?.restart_required) throw new WidevineSetupError('widevine_restart');
        if (status?.phase === 'error') throw new WidevineSetupError('widevine_service_error');
        await wait(Math.min(1000, Math.max(0, timeout - (now() - started))), signal);
    }
    if (signal.aborted) throw new DOMException('Cancelled', 'AbortError');
    throw new WidevineSetupError('widevine_unavailable');
}

export function widevineMessage(status: WidevineStatus | null) {
    switch (status?.phase) {
        case 'downloading':
            return [
                'Download di Widevine in corso.',
                'Lume sta scaricando da Google il componente per i canali protetti.',
            ];
        case 'installing':
            return [
                'Installazione di Widevine.',
                'Il download è terminato. Stiamo preparando il componente.',
            ];
        case 'installed':
            return [
                'Verifica dei canali protetti.',
                'Il componente è installato. Verifichiamo che sia pronto per la riproduzione.',
            ];
        case 'error':
            return [
                'Download di Widevine non riuscito.',
                'Controlla la connessione e premi Riprova.',
            ];
        case 'unavailable':
            return [
                'Prepariamo i canali protetti.',
                'Solo al primo avvio può servire qualche minuto. Lascia Lume aperta.',
            ];
        default:
            return [
                'Controllo di Widevine.',
                'Verifichiamo il componente necessario per i canali protetti.',
            ];
    }
}
