import { useEffect, useRef, useState } from 'react';
import { api, apiFetch } from '../api';
import { refreshCatalog, restoreBackup } from '../app';
import { errorMessage, refreshLibrary, type PersonalSource } from '../personal';
interface BackupJob {
    result?: { playlists: number; kept: number; favorites: number; watch: number; skipped: number };
    id: string;
    phase: string;
    error: string;
    message?: string;
    url?: string;
    created?: number;
    sources?: PersonalSource[];
    favorites?: number;
    series?: number;
    watch?: number;
}
const categories = [
    ['PLAYLISTS', 'Liste personali'],
    ['PERSONAL', 'Preferiti e personalizzazioni'],
    ['WATCH', 'Avanzamenti dei video e dell’archivio'],
    ['PREFERENCES', 'Preferenze di riproduzione'],
] as const;
export function BackupSettings() {
    const [mode, setMode] = useState<'export' | 'import'>('export');
    const [password, setPassword] = useState('');
    const [file, setFile] = useState<File>();
    const [job, setJob] = useState<BackupJob>();
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState('');
    const [notice, setNotice] = useState('');
    const [chosen, setChosen] = useState<string[]>([]);
    const [selection, setSelection] = useState<string[]>(categories.map(([key]) => key));
    const alive = useRef(true);
    const controller = useRef<AbortController | null>(null);
    const current = useRef<BackupJob | undefined>(undefined);
    useEffect(() => {
        alive.current = true;
        return () => {
            alive.current = false;
            controller.current?.abort();
            if (current.current)
                void api('/api/library/backup/cancel', { id: current.current.id }).catch(() => {});
        };
    }, []);
    useEffect(() => {
        current.current = job;
    }, [job]);
    useEffect(() => {
        if (!job || ['ready', 'exported', 'restored', 'error', 'cancelled'].includes(job.phase))
            return;
        let stopped = false;
        let timer: ReturnType<typeof setTimeout>;
        const poll = async () => {
            try {
                const next = await api<BackupJob>('/api/library/backup/status', { id: job.id });
                if (!stopped) {
                    setJob(next);
                    if (next.phase === 'ready') setChosen(next.sources?.map((s) => s.id) || []);
                }
            } catch (e) {
                if (!stopped) setError(errorMessage(e));
            }
            if (!stopped) timer = setTimeout(poll, 700);
        };
        timer = setTimeout(poll, 700);
        return () => {
            stopped = true;
            clearTimeout(timer);
        };
    }, [job?.id, job?.phase]);
    const close = async () => {
        controller.current?.abort();
        if (job) {
            try {
                await api('/api/library/backup/cancel', { id: job.id });
            } catch (e) {
                setError(errorMessage(e));
                return;
            }
        }
        setJob(undefined);
        setPassword('');
        setBusy(false);
    };
    const start = async () => {
        setError('');
        setNotice('');
        setBusy(true);
        const abort = new AbortController();
        controller.current = abort;
        try {
            if (password.length < 8 || password.length > 128)
                throw { code: 'Usa una password da 8 a 128 caratteri.' };
            if (mode === 'export') {
                const next = await api<BackupJob>(
                    '/api/library/backup/export',
                    { password },
                    { signal: abort.signal },
                );
                if (alive.current) setJob(next);
            } else {
                if (!file) throw { code: 'Scegli un file .lumebackup.' };
                if (file.size > 2 * 1024 ** 3 + 1024 ** 2)
                    throw { code: 'Il backup supera il limite di 2 GiB.' };
                const next = await api<BackupJob>(
                    '/api/library/backup/input',
                    {},
                    { signal: abort.signal },
                );
                current.current = next;
                setJob(next);
                const response = await apiFetch(`/api/library/backup/upload/${next.id}`, {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/octet-stream' },
                    body: file,
                    signal: abort.signal,
                });
                const uploaded = (await response.json()) as BackupJob;
                if (!response.ok || uploaded.phase === 'error')
                    throw { code: uploaded.error || 'Caricamento non riuscito.' };
                const prepared = await api<BackupJob>(
                    '/api/library/backup/prepare',
                    { id: next.id, password },
                    { signal: abort.signal },
                );
                if (alive.current) setJob(prepared);
            }
            setPassword('');
        } catch (e) {
            if (alive.current && (e as Error).name !== 'AbortError') setError(errorMessage(e));
        } finally {
            if (alive.current) setBusy(false);
        }
    };
    const restore = async () => {
        if (!job) return;
        setBusy(true);
        setError('');
        try {
            await restoreBackup({ id: job.id, categories: selection, sources: chosen });
            await Promise.all([refreshLibrary(), refreshCatalog()]);
            const completed = await api<BackupJob>('/api/library/backup/status', { id: job.id });
            setJob(completed);
            setNotice(
                completed.result
                    ? `Ripristino completato: ${completed.result.playlists} liste aggiunte, ${completed.result.kept} già presenti, ${completed.result.favorites} preferiti e ${completed.result.watch} avanzamenti recuperati.${completed.result.skipped ? ` ${completed.result.skipped} riferimenti già presenti o senza un contenuto corrispondente sono stati ignorati.` : ''}`
                    : 'Ripristino completato. I dati già presenti sono stati conservati.',
            );
        } catch (e) {
            setError(errorMessage(e));
        } finally {
            setBusy(false);
        }
    };
    const running =
        busy ||
        (!!job && !['ready', 'exported', 'restored', 'error', 'cancelled'].includes(job.phase));
    return (
        <section className="backup-settings" aria-label="Backup e ripristino">
            <h2>Backup e ripristino</h2>
            <div className="settings-card backup-content">
                <p>
                    Porta le tue liste, i preferiti e gli avanzamenti tra desktop e Android. Il file
                    è protetto da una password e contiene anche gli indirizzi e le credenziali delle
                    liste.
                </p>
                {!job && (
                    <>
                        <div className="segmented" aria-label="Operazione backup">
                            <button
                                aria-pressed={mode === 'export'}
                                disabled={busy}
                                onClick={() => setMode('export')}
                            >
                                Crea backup
                            </button>
                            <button
                                aria-pressed={mode === 'import'}
                                disabled={busy}
                                onClick={() => setMode('import')}
                            >
                                Ripristina
                            </button>
                        </div>
                        {mode === 'import' && (
                            <label className="library-field">
                                <span>File Lume</span>
                                <input
                                    type="file"
                                    accept=".lumebackup"
                                    onChange={(e) => setFile(e.target.files?.[0])}
                                    disabled={busy}
                                />
                            </label>
                        )}
                        <label className="library-field">
                            <span>
                                {mode === 'export' ? 'Scegli una password' : 'Password del backup'}
                            </span>
                            <input
                                type="password"
                                autoComplete="new-password"
                                minLength={8}
                                maxLength={128}
                                value={password}
                                onChange={(e) => setPassword(e.target.value)}
                                disabled={busy}
                            />
                            <small>
                                Da 8 a 128 caratteri. Conserva la password: serve per aprire il
                                backup.
                            </small>
                        </label>
                        <button
                            className="button primary"
                            disabled={busy || password.length < 8 || (mode === 'import' && !file)}
                            onClick={() => void start()}
                        >
                            {busy
                                ? 'Caricamento…'
                                : mode === 'export'
                                  ? 'Prepara backup'
                                  : 'Verifica backup'}
                        </button>
                    </>
                )}
                {job && (
                    <div aria-live="polite">
                        {running && (
                            <p role="status">
                                {job.phase === 'verifying'
                                    ? 'Verifico password e integrità…'
                                    : job.phase === 'reading'
                                      ? 'Preparo le liste e gli avanzamenti…'
                                      : job.phase === 'encrypting'
                                        ? 'Proteggo il backup…'
                                        : job.message || 'Operazione in corso…'}
                            </p>
                        )}
                        {job.phase === 'exported' && (
                            <>
                                <p>Il backup cifrato è pronto.</p>
                                <a
                                    className="button primary"
                                    href={job.url}
                                    download="Lume.lumebackup"
                                >
                                    Salva backup
                                </a>
                            </>
                        )}
                        {job.phase === 'ready' && (
                            <>
                                <h3>Contenuto del backup</h3>
                                <p>
                                    {job.created
                                        ? new Date(job.created).toLocaleString('it-IT')
                                        : ''}{' '}
                                    · {job.favorites || 0} preferiti · {job.series || 0} serie
                                    salvate · {job.watch || 0} avanzamenti
                                </p>
                                <fieldset disabled={busy}>
                                    <legend>Scegli cosa ripristinare</legend>
                                    {categories.map(([key, label]) => (
                                        <label className="library-check" key={key}>
                                            <input
                                                type="checkbox"
                                                checked={selection.includes(key)}
                                                onChange={(e) =>
                                                    setSelection(
                                                        e.target.checked
                                                            ? [...selection, key]
                                                            : selection.filter((v) => v !== key),
                                                    )
                                                }
                                            />
                                            {label}
                                        </label>
                                    ))}
                                </fieldset>
                                {selection.includes('PLAYLISTS') && (
                                    <fieldset disabled={busy}>
                                        <legend>Liste da aggiungere</legend>
                                        {job.sources?.map((s) => (
                                            <label className="library-check" key={s.id}>
                                                <input
                                                    type="checkbox"
                                                    checked={chosen.includes(s.id)}
                                                    onChange={(e) =>
                                                        setChosen(
                                                            e.target.checked
                                                                ? [...chosen, s.id]
                                                                : chosen.filter(
                                                                      (id) => id !== s.id,
                                                                  ),
                                                        )
                                                    }
                                                />
                                                <span>
                                                    {s.name}
                                                    <small>
                                                        {s.report.count.toLocaleString('it-IT')}{' '}
                                                        voci
                                                    </small>
                                                </span>
                                            </label>
                                        ))}
                                        {!job.sources?.length && (
                                            <p>Nessuna lista in questo backup.</p>
                                        )}
                                    </fieldset>
                                )}
                                <p>
                                    Il ripristino aggiunge i dati mancanti. Le liste con lo stesso
                                    identificativo restano nella versione già presente sul computer.
                                </p>
                                <button
                                    className="button primary"
                                    disabled={busy || !selection.length}
                                    onClick={() => void restore()}
                                >
                                    {busy ? 'Ripristino…' : 'Ripristina selezione'}
                                </button>
                            </>
                        )}
                        {job.error && <p role="alert">{job.error}</p>}
                        <button
                            className="text-button"
                            disabled={busy && job.phase === 'ready'}
                            onClick={() => void close()}
                        >
                            {running ? 'Annulla' : 'Chiudi'}
                        </button>
                    </div>
                )}
                {error && (
                    <p className="form-error" role="alert">
                        {error}
                    </p>
                )}
                {notice && <p role="status">{notice}</p>}
            </div>
        </section>
    );
}
