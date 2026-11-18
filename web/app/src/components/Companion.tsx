import { useEffect, useRef, useState } from 'react';
import { closeDialog, navigate, playback } from '../app';
import { companion, remoteTv, type CompanionJob, type Television } from '../companion';
import { errorMessage, library } from '../personal';
import { useStore } from '../store';
import { Icon } from './common';
function useJob(job: CompanionJob | undefined, setJob: (j: CompanionJob) => void) {
    useEffect(() => {
        if (
            !job ||
            ['ready', 'paired', 'started', 'imported', 'cancelled', 'error'].includes(job.phase)
        )
            return;
        let stopped = false;
        let timer: ReturnType<typeof setTimeout>;
        const controller = new AbortController();
        const poll = async () => {
            try {
                const next = await companion<CompanionJob>(
                    'status',
                    { id: job.id },
                    controller.signal,
                );
                if (!stopped) setJob(next);
            } catch (e) {
                if (!stopped) setJob({ ...job, phase: 'error', error: errorMessage(e) });
            }
            if (!stopped) timer = setTimeout(poll, 700);
        };
        timer = setTimeout(poll, 500);
        return () => {
            stopped = true;
            controller.abort();
            clearTimeout(timer);
        };
    }, [job?.id, job?.phase, setJob]);
}
function useDevices() {
    const [tvs, setTvs] = useState<Television[]>([]);
    const [error, setError] = useState('');
    const load = async (signal?: AbortSignal) => {
        try {
            setTvs(
                (await companion<{ televisions: Television[] }>('devices', {}, signal))
                    .televisions || [],
            );
        } catch (e) {
            if (!signal?.aborted) setError(errorMessage(e));
        }
    };
    useEffect(() => {
        const c = new AbortController();
        void load(c.signal);
        return () => c.abort();
    }, []);
    return { tvs, error, reload: load };
}
export function CompanionSettings() {
    const devices = useDevices();
    const personal = useStore(library);
    const [add, setAdd] = useState(false);
    const [invite, setInvite] = useState('');
    const [host, setHost] = useState('');
    const [port, setPort] = useState('');
    const [code, setCode] = useState('');
    const [found, setFound] = useState<Television[]>([]);
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState('');
    const [notice, setNotice] = useState('');
    const [job, setJob] = useState<CompanionJob>();
    const [destination, setDestination] = useState('');
    const [source, setSource] = useState('');
    const [name, setName] = useState('');
    const [consent, setConsent] = useState(false);
    const latest = useRef<CompanionJob | undefined>(undefined);
    latest.current = job;
    useJob(job, setJob);
    useEffect(
        () => () => {
            if (latest.current && !['imported', 'paired'].includes(latest.current.phase))
                void companion('cancel', { id: latest.current.id }).catch(() => {});
        },
        [],
    );
    useEffect(() => {
        if (job?.phase === 'paired') {
            void devices.reload();
            void companion('cancel', { id: job.id }).catch(() => {});
            setJob(undefined);
            setCode('');
            setInvite('');
            setAdd(false);
            setNotice('TV associata. Puoi aprire il telecomando o inviare un contenuto.');
        }
    }, [job?.phase]);
    const run = async (action: () => Promise<void>) => {
        setBusy(true);
        setError('');
        setNotice('');
        try {
            await action();
        } catch (e) {
            setError(errorMessage(e));
        } finally {
            setBusy(false);
        }
    };
    const cancel = async () => {
        if (job) await companion('cancel', { id: job.id });
        setJob(undefined);
    };
    const occupied = !!job && !['error', 'cancelled', 'paired', 'imported'].includes(job.phase);
    return (
        <section className="companion-settings" aria-label="Collegamento alla TV">
            <h2>La tua TV</h2>
            <div className="settings-card companion-content">
                <p>
                    Controlla Lume sulla TV e invia canali, film e serie dalla stessa rete Wi-Fi o
                    Ethernet.
                </p>
                {devices.tvs.map((tv) => (
                    <div className="companion-device" key={tv.id}>
                        <Icon name="tv" />
                        <span>
                            <strong>{tv.name}</strong>
                            <small>
                                {tv.host}:{tv.port}
                            </small>
                        </span>
                        <button className="button secondary" onClick={() => remoteTv(tv.id)}>
                            Telecomando
                        </button>
                        <button
                            className="text-button"
                            disabled={busy || occupied}
                            onClick={() =>
                                void run(async () => {
                                    await companion('forget', { tv: tv.id });
                                    await devices.reload();
                                })
                            }
                        >
                            Dimentica
                        </button>
                    </div>
                ))}
                {!devices.tvs.length && (
                    <p>Nessuna TV associata. Apri “Collega telefono” in Lume sulla TV.</p>
                )}
                {!add && (
                    <button
                        className="button secondary"
                        disabled={busy || occupied || devices.tvs.length >= 8}
                        onClick={() => {
                            setJob(undefined);
                            setAdd(true);
                        }}
                    >
                        Associa una TV
                    </button>
                )}
                {add && (
                    <div className="companion-pair">
                        <p>
                            Usa l’indirizzo e il codice mostrati sulla TV, oppure incolla il testo
                            del suo QR Lume.
                        </p>
                        <button
                            className="text-button"
                            disabled={busy || occupied}
                            onClick={() =>
                                void run(async () => {
                                    const data = await companion<{ televisions: Television[] }>(
                                        'discover',
                                    );
                                    setFound(data.televisions);
                                    if (!data.televisions.length)
                                        setNotice(
                                            'Nessuna TV trovata. Puoi inserire l’indirizzo mostrato sulla TV.',
                                        );
                                })
                            }
                        >
                            {busy ? 'Ricerca…' : 'Cerca TV nella rete'}
                        </button>
                        {found.map((tv) => (
                            <button
                                className="button secondary"
                                key={`${tv.host}:${tv.port}`}
                                onClick={() => {
                                    setHost(tv.host);
                                    setPort(String(tv.port));
                                }}
                            >
                                {tv.name} · {tv.host}
                            </button>
                        ))}
                        <div className="companion-address">
                            <label className="library-field">
                                <span>Indirizzo IPv4 della TV</span>
                                <input
                                    value={host}
                                    placeholder="192.168.1.20"
                                    maxLength={15}
                                    autoComplete="off"
                                    onChange={(e) => setHost(e.target.value)}
                                    disabled={occupied}
                                />
                            </label>
                            <label className="library-field">
                                <span>Porta</span>
                                <input
                                    type="number"
                                    min={1}
                                    max={65535}
                                    value={port}
                                    onChange={(e) => setPort(e.target.value)}
                                    disabled={occupied}
                                />
                            </label>
                        </div>
                        <label className="library-field">
                            <span>Codice di associazione</span>
                            <input
                                value={code}
                                autoComplete="off"
                                maxLength={24}
                                spellCheck={false}
                                onChange={(e) => setCode(e.target.value)}
                                disabled={occupied}
                            />
                        </label>
                        <details>
                            <summary>Ho un invito QR</summary>
                            <label className="library-field">
                                <span>Testo dell’invito Lume</span>
                                <input
                                    value={invite}
                                    type="password"
                                    autoComplete="off"
                                    maxLength={512}
                                    onChange={(e) => setInvite(e.target.value)}
                                    disabled={occupied}
                                />
                            </label>
                        </details>
                        <div className="video-actions">
                            <button
                                className="button primary"
                                disabled={
                                    busy ||
                                    occupied ||
                                    (!invite.trim() && (!host || !port || !code))
                                }
                                onClick={() =>
                                    void run(async () => {
                                        if (job) await cancel();
                                        const result = await companion<CompanionJob>('pair', {
                                            invite,
                                            endpoint: { host, port: Number(port) },
                                            code,
                                        });
                                        setJob(result);
                                        setCode('');
                                        setInvite('');
                                    })
                                }
                            >
                                Associa TV
                            </button>
                            <button
                                className="text-button"
                                disabled={busy}
                                onClick={() =>
                                    void run(async () => {
                                        await cancel();
                                        setAdd(false);
                                        setCode('');
                                        setInvite('');
                                    })
                                }
                            >
                                Annulla
                            </button>
                        </div>
                    </div>
                )}
                {devices.tvs.length > 0 && personal.sources.length > 0 && (
                    <details className="companion-transfer">
                        <summary>Invia una lista alla TV</summary>
                        <p>
                            La lista comprende indirizzi, header e credenziali. Verrà inviata alla
                            TV scelta tramite il collegamento cifrato; preferiti e cronologia
                            restano sul computer.
                        </p>
                        <label className="library-field">
                            <span>TV di destinazione</span>
                            <select
                                value={destination}
                                disabled={occupied}
                                onChange={(e) => setDestination(e.target.value)}
                            >
                                <option value="">Scegli TV</option>
                                {devices.tvs.map((tv) => (
                                    <option key={tv.id} value={tv.id}>
                                        {tv.name}
                                    </option>
                                ))}
                            </select>
                        </label>
                        <label className="library-field">
                            <span>Lista da inviare</span>
                            <select
                                value={source}
                                disabled={occupied}
                                onChange={(e) => {
                                    const s = personal.sources.find((s) => s.id === e.target.value);
                                    setSource(e.target.value);
                                    setName(s?.name || '');
                                    setConsent(false);
                                }}
                            >
                                <option value="">Scegli lista</option>
                                {personal.sources.map((s) => (
                                    <option key={s.id} value={s.id}>
                                        {s.name}
                                    </option>
                                ))}
                            </select>
                        </label>
                        <label className="library-check">
                            <input
                                type="checkbox"
                                checked={consent}
                                disabled={occupied}
                                onChange={(e) => setConsent(e.target.checked)}
                            />
                            <span>Invia anche le credenziali della lista alla TV selezionata</span>
                        </label>
                        <button
                            className="button secondary"
                            disabled={busy || occupied || !destination || !source || !consent}
                            onClick={() =>
                                void run(async () => {
                                    if (job) await cancel();
                                    setJob(
                                        await companion<CompanionJob>('transfer', {
                                            tv: destination,
                                            source,
                                        }),
                                    );
                                })
                            }
                        >
                            Invia e verifica lista
                        </button>
                    </details>
                )}
                {job && (
                    <div aria-live="polite">
                        {job.phase === 'waiting' && (
                            <p role="status">Conferma l’associazione su {job.name || 'TV'}.</p>
                        )}
                        {['working', 'uploading', 'preparing', 'importing'].includes(job.phase) && (
                            <p role="status">
                                {job.phase === 'uploading' && job.size
                                    ? `Invio lista: ${Math.round(((job.received || 0) / job.size) * 100)}%`
                                    : job.phase === 'importing'
                                      ? 'Importo la lista sulla TV…'
                                      : job.phase === 'preparing'
                                        ? 'La TV verifica la lista…'
                                        : 'Collegamento in corso…'}
                            </p>
                        )}
                        {job.phase === 'ready' && job.preview && (
                            <>
                                <h3>Lista pronta sulla TV</h3>
                                <p>
                                    {job.preview.count.toLocaleString('it-IT')} voci ·{' '}
                                    {job.preview.sample.slice(0, 4).join(', ')}
                                </p>
                                <label className="library-field">
                                    <span>Nome sulla TV</span>
                                    <input
                                        maxLength={100}
                                        value={name}
                                        onChange={(e) => setName(e.target.value)}
                                    />
                                </label>
                                <button
                                    className="button primary"
                                    disabled={busy || !name.trim()}
                                    onClick={() =>
                                        void run(async () =>
                                            setJob(
                                                await companion<CompanionJob>('import', {
                                                    id: job.id,
                                                    name,
                                                }),
                                            ),
                                        )
                                    }
                                >
                                    Importa sulla TV
                                </button>
                            </>
                        )}
                        {job.phase === 'imported' && (
                            <p role="status">
                                Lista importata sulla TV. Ora puoi inviare i suoi canali e video.
                            </p>
                        )}
                        {job.error && <p role="alert">{job.error}</p>}
                        {!['paired', 'imported'].includes(job.phase) && (
                            <button
                                className="text-button"
                                disabled={busy || job.phase === 'importing'}
                                onClick={() => void run(cancel)}
                            >
                                Chiudi operazione
                            </button>
                        )}
                    </div>
                )}
                {(error || devices.error) && <p role="alert">{error || devices.error}</p>}
                {notice && <p role="status">{notice}</p>}
            </div>
        </section>
    );
}
export function WatchTvDialog({ id, position }: { id: string; position: number }) {
    const devices = useDevices();
    const [tv, setTv] = useState('');
    const [job, setJob] = useState<CompanionJob>();
    const [error, setError] = useState('');
    const [busy, setBusy] = useState(false);
    const latest = useRef<CompanionJob | undefined>(undefined);
    latest.current = job;
    useJob(job, setJob);
    useEffect(() => {
        if (!tv && devices.tvs.length) setTv(devices.tvs[0].id);
    }, [devices.tvs]);
    useEffect(
        () => () => {
            if (latest.current) void companion('cancel', { id: latest.current.id }).catch(() => {});
        },
        [],
    );
    useEffect(() => {
        if (job?.phase === 'started' && playback.store.get().current === id)
            void playback.stop().then(() => remoteTv(tv));
    }, [job?.phase, id, tv]);
    const run = async (action: () => Promise<void>) => {
        setBusy(true);
        setError('');
        try {
            await action();
        } catch (e) {
            setError(errorMessage(e));
        } finally {
            setBusy(false);
        }
    };
    const running = !!job && !['ready', 'error', 'started', 'cancelled'].includes(job.phase);
    return (
        <>
            <div className="dialog-top">
                <Icon name="tv" />
                <button className="icon-button" aria-label="Chiudi" onClick={closeDialog}>
                    <Icon name="close" />
                </button>
            </div>
            <h2 id="dialog-title">Guarda sulla TV</h2>
            <p>
                Verifica che la TV abbia lo stesso contenuto, poi scegli quando avviarlo. La
                riproduzione sul computer continua fino alla conferma della TV.
            </p>
            {devices.tvs.length ? (
                <label className="library-field">
                    <span>TV</span>
                    <select
                        value={tv}
                        disabled={running || busy}
                        onChange={(e) => {
                            if (job) void companion('cancel', { id: job.id }).catch(() => {});
                            setJob(undefined);
                            setTv(e.target.value);
                        }}
                    >
                        {devices.tvs.map((t) => (
                            <option key={t.id} value={t.id}>
                                {t.name}
                            </option>
                        ))}
                    </select>
                </label>
            ) : (
                <button
                    className="button primary"
                    onClick={() => {
                        closeDialog();
                        navigate('settings');
                    }}
                >
                    Associa una TV nelle impostazioni
                </button>
            )}
            {devices.tvs.length > 0 && (!job || ['error', 'cancelled'].includes(job.phase)) && (
                <button
                    className="button primary"
                    disabled={busy || !tv}
                    onClick={() =>
                        void run(async () => {
                            if (job) await companion('cancel', { id: job.id });
                            setJob(
                                await companion<CompanionJob>('resolve', { tv, channel_id: id }),
                            );
                        })
                    }
                >
                    Verifica sulla TV
                </button>
            )}
            {running && (
                <p role="status">
                    {job?.phase === 'starting'
                        ? 'Attendo la conferma di riproduzione dalla TV…'
                        : 'Cerco il contenuto sulla TV…'}
                </p>
            )}
            {job?.phase === 'ready' && (
                <>
                    <h3>{job.title}</h3>
                    <button
                        className="button primary"
                        disabled={busy}
                        onClick={() =>
                            void run(async () => {
                                const current = playback.store.get();
                                const point = current.current === id ? current.position : position;
                                setJob(
                                    await companion<CompanionJob>('start', {
                                        id: job.id,
                                        channel_id: id,
                                        position: job.vod
                                            ? Math.max(0, Math.round(point * 1000))
                                            : 0,
                                    }),
                                );
                            })
                        }
                    >
                        Avvia sulla TV
                    </button>
                </>
            )}
            {job?.phase === 'started' && (
                <>
                    <p role="status">Riproduzione avviata sulla TV.</p>
                    <button className="button secondary" onClick={() => remoteTv(tv)}>
                        Apri telecomando
                    </button>
                </>
            )}
            {(error || job?.error || devices.error) && (
                <p role="alert">{error || job?.error || devices.error}</p>
            )}
            {job?.phase === 'error' && id.startsWith('personal_') && (
                <button
                    className="text-button"
                    onClick={() => {
                        closeDialog();
                        navigate('settings');
                    }}
                >
                    Gestisci TV e invio liste
                </button>
            )}
        </>
    );
}
interface Track {
    key: string;
    label: string;
    selected: boolean;
}
interface TvState {
    instance: string;
    sequence: number;
    session: string;
    title: string;
    programme: string;
    active: boolean;
    wantsToPlay: boolean;
    playing: boolean;
    mode: string;
    seekable: boolean;
    position: number;
    duration: number;
    nextEpisode: string;
    transitioning: boolean;
    options: {
        revision: number;
        audio: Track[];
        subtitles: Track[];
        video: Track[];
        audioAutomatic: boolean;
        subtitlesAutomatic: boolean;
        subtitlesDisabled: boolean;
        videoAutomatic: boolean;
        sleepRemaining: number;
    };
}
export function RemoteDialog({ tv }: { tv: string }) {
    const [state, setState] = useState<TvState>();
    const [error, setError] = useState('');
    const [busy, setBusy] = useState(false);
    const generation = useRef(0);
    const alive = useRef(true);
    useEffect(() => {
        alive.current = true;
        let timer: ReturnType<typeof setTimeout>;
        const controller = new AbortController();
        const poll = async () => {
            const own = ++generation.current;
            try {
                const value = await companion<TvState>('state', { tv }, controller.signal);
                if (alive.current && own === generation.current) {
                    setState(value);
                    setError('');
                }
            } catch (e) {
                if (alive.current && !controller.signal.aborted && own === generation.current)
                    setError(errorMessage(e));
            }
            if (alive.current) timer = setTimeout(poll, 2000);
        };
        void poll();
        return () => {
            alive.current = false;
            controller.abort();
            clearTimeout(timer);
        };
    }, [tv]);
    const command = async (action: string, args: object = {}) => {
        if (!state || busy) return;
        const own = ++generation.current;
        setBusy(true);
        setError('');
        try {
            const value = await companion<TvState>('command', {
                tv,
                session: state.session,
                action,
                args,
                command_id: crypto.randomUUID(),
            });
            if (alive.current && own === generation.current) setState(value);
        } catch (e) {
            if (alive.current) setError(errorMessage(e));
        } finally {
            if (alive.current) setBusy(false);
        }
    };
    return (
        <>
            <div className="dialog-top">
                <Icon name="tv" />
                <button className="icon-button" aria-label="Chiudi" onClick={closeDialog}>
                    <Icon name="close" />
                </button>
            </div>
            <h2 id="dialog-title">Telecomando Lume</h2>
            <p aria-live="polite">
                {state?.title || 'Lume sulla TV'}
                {state?.programme && <small className="programme-meta">{state.programme}</small>}
            </p>
            {!state && !error && <p role="status">Collegamento alla TV…</p>}
            <div className="remote-navigation" aria-label="Navigazione TV">
                {[
                    ['up', 'Su'],
                    ['left', 'Sinistra'],
                    ['ok', 'OK'],
                    ['right', 'Destra'],
                    ['down', 'Giù'],
                ].map(([key, label]) => (
                    <button
                        key={key}
                        className={`button secondary remote-${key}`}
                        disabled={!state || busy}
                        onClick={() => void command('navigate', { key })}
                    >
                        {label}
                    </button>
                ))}
            </div>
            <div className="video-actions">
                {[
                    ['back', 'Indietro'],
                    ['home', 'Home'],
                    ['options', 'Opzioni'],
                ].map(([key, label]) => (
                    <button
                        key={key}
                        className="text-button"
                        disabled={!state || busy}
                        onClick={() => void command('navigate', { key })}
                    >
                        {label}
                    </button>
                ))}
            </div>
            {state?.active && (
                <>
                    <div className="video-actions">
                        <button
                            className="button primary"
                            disabled={busy || state.transitioning}
                            onClick={() => void command(state.wantsToPlay ? 'pause' : 'play')}
                        >
                            <Icon name={state.wantsToPlay ? 'pause' : 'play'} />
                            {state.wantsToPlay ? 'Pausa' : 'Riprendi'}
                        </button>
                        <button
                            className="button secondary"
                            disabled={busy}
                            onClick={() => void command('stop')}
                        >
                            Ferma
                        </button>
                        {state.mode === 'LIVE' && (
                            <>
                                <button
                                    className="text-button"
                                    disabled={busy || state.transitioning}
                                    onClick={() => void command('channelPrevious')}
                                >
                                    Canale precedente
                                </button>
                                <button
                                    className="text-button"
                                    disabled={busy || state.transitioning}
                                    onClick={() => void command('channelNext')}
                                >
                                    Canale successivo
                                </button>
                            </>
                        )}
                    </div>
                    {state.seekable && (
                        <div className="video-actions">
                            <button
                                className="text-button"
                                disabled={busy}
                                onClick={() => void command('seekBy', { delta: -10000 })}
                            >
                                Indietro 10 s
                            </button>
                            <button
                                className="text-button"
                                disabled={busy}
                                onClick={() => void command('seekBy', { delta: 30000 })}
                            >
                                Avanti 30 s
                            </button>
                        </div>
                    )}
                    {state.mode === 'ARCHIVE' && (
                        <button
                            className="text-button"
                            disabled={busy}
                            onClick={() => void command('live')}
                        >
                            Torna alla diretta
                        </button>
                    )}
                    {state.nextEpisode && (
                        <button
                            className="text-button"
                            disabled={busy}
                            onClick={() => void command('nextEpisode')}
                        >
                            Prossimo episodio: {state.nextEpisode}
                        </button>
                    )}
                    {state.options && (
                        <details>
                            <summary>Tracce e qualità sulla TV</summary>
                            {(
                                [
                                    ['audio', 'Audio', 1, state.options.audioAutomatic],
                                    [
                                        'subtitles',
                                        'Sottotitoli',
                                        3,
                                        state.options.subtitlesAutomatic,
                                    ],
                                    ['video', 'Qualità', 2, state.options.videoAutomatic],
                                ] as const
                            ).map(([key, label, type, automatic]) => (
                                <label className="library-field" key={key}>
                                    <span>{label}</span>
                                    <select
                                        disabled={busy}
                                        value={
                                            key === 'subtitles' && state.options.subtitlesDisabled
                                                ? 'off'
                                                : automatic
                                                  ? 'auto'
                                                  : state.options[key].find((t) => t.selected)
                                                        ?.key || 'auto'
                                        }
                                        onChange={(e) =>
                                            void command(
                                                e.target.value === 'off' ? 'subtitlesOff' : 'track',
                                                {
                                                    type,
                                                    key: e.target.value,
                                                    revision: state.options.revision,
                                                },
                                            )
                                        }
                                    >
                                        <option value="auto">Automatico</option>
                                        {key === 'subtitles' && (
                                            <option value="off">Disattivati</option>
                                        )}
                                        {state.options[key].map((t) => (
                                            <option key={t.key} value={t.key}>
                                                {t.label}
                                            </option>
                                        ))}
                                    </select>
                                </label>
                            ))}
                        </details>
                    )}
                </>
            )}
            {error && <p role="alert">{error}</p>}
        </>
    );
}
