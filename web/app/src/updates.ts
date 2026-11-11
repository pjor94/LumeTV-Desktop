import { api } from './api';
import { createStore } from './store';

export interface UpdateStatus {
    current_version: string;
    phase:
        | 'idle'
        | 'checking'
        | 'up_to_date'
        | 'available'
        | 'downloading'
        | 'ready'
        | 'installing'
        | 'opened'
        | 'error';
    available_version: string | null;
    notes: string;
    downloaded: number;
    total: number;
    error: string | null;
    settings: { automatic: boolean; preview: boolean };
    last_checked: number | null;
    next_check_at: number;
    install_mode: 'replace' | 'system' | 'open';
    last_install_failed: boolean;
}
export const updates = createStore<UpdateStatus>({
    current_version: '',
    phase: 'idle',
    available_version: null,
    notes: '',
    downloaded: 0,
    total: 0,
    error: null,
    settings: { automatic: true, preview: false },
    last_checked: null,
    next_check_at: 0,
    install_mode: 'open',
    last_install_failed: false,
});
export function updateBusy(status: UpdateStatus) {
    return ['checking', 'downloading', 'installing'].includes(status.phase);
}
export function downloadPercent(downloaded: number, total: number) {
    return total > 0 ? Math.min(100, Math.max(0, Math.floor((downloaded / total) * 100))) : 0;
}
export function updateError(error: string | null) {
    if (!error) return '';
    return (
        (
            {
                network:
                    'Non riesco a raggiungere il servizio. Controlla la connessione e riprova.',
                rate_limited: 'Il servizio richiede una pausa. Il controllo riprenderà più tardi.',
                signature:
                    'La firma del pacchetto non è valida. Riprova il controllo degli aggiornamenti.',
                checksum: 'Il download è incompleto o danneggiato. Scarica di nuovo il pacchetto.',
                metadata: 'I dati della nuova versione non sono validi. Riprova più tardi.',
                disk: 'Non riesco a salvare il pacchetto. Controlla lo spazio libero e riprova.',
                install:
                    'Non riesco ad avviare l’installazione. Riprova o usa il pacchetto della release.',
                unsupported: 'Gli aggiornamenti per questo sistema non sono disponibili.',
                local: 'Non riesco a contattare l’app. Riprova tra poco.',
            } as Record<string, string>
        )[error] || 'L’operazione non è riuscita. Riprova.'
    );
}
let revision = 0;
export async function updateAction(
    action: 'check' | 'download' | 'cancel' | 'install' | 'settings',
    body: unknown = {},
) {
    const requestedRevision = ++revision;
    try {
        const status = await api<UpdateStatus>(`/api/updates/${action}`, body);
        if (revision === requestedRevision) updates.set(status);
    } catch (error) {
        if (revision === requestedRevision)
            updates.set({ error: (error as { code?: string }).code || 'local' });
    }
}
export function connectUpdates() {
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let request = false;
    const tick = async () => {
        if (stopped || request) return;
        clearTimeout(timer);
        request = true;
        const requestedRevision = revision;
        try {
            const status = await api<UpdateStatus>('/api/updates');
            if (stopped || revision !== requestedRevision) return;
            updates.set(status);
            if (
                document.visibilityState === 'visible' &&
                status.settings.automatic &&
                !updateBusy(status) &&
                Date.now() / 1000 >= status.next_check_at
            ) {
                const checked = await api<UpdateStatus>('/api/updates/check', { manual: false });
                if (!stopped && revision === requestedRevision) updates.set(checked);
            }
        } catch {
            if (!stopped && revision === requestedRevision) updates.set({ error: 'local' });
        } finally {
            request = false;
            if (!stopped)
                timer = setTimeout(() => void tick(), updateBusy(updates.get()) ? 1000 : 60_000);
        }
    };
    const wake = () => {
        if (document.visibilityState === 'visible') void tick();
    };
    const unsubscribe = updates.subscribe(() => {
        if (updateBusy(updates.get()) && !request) {
            clearTimeout(timer);
            timer = setTimeout(() => void tick(), 1000);
        }
    });
    document.addEventListener('visibilitychange', wake);
    window.addEventListener('online', wake);
    void tick();
    return () => {
        stopped = true;
        clearTimeout(timer);
        unsubscribe();
        document.removeEventListener('visibilitychange', wake);
        window.removeEventListener('online', wake);
    };
}
