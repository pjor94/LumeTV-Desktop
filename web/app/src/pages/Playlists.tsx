import {
    cloneElement,
    isValidElement,
    useEffect,
    useId,
    useRef,
    useState,
    type ReactNode,
    type ReactElement,
} from 'react';
import { api, apiFetch } from '../api';
import { navigate, playback } from '../app';
import { date, time } from '../domain';
import { useStore } from '../store';
import {
    defaultSettings,
    errorMessage,
    library,
    librarySelection,
    personalRadioSelection,
    refreshLibrary,
    type Access,
    type ImportJob,
    type LibraryPage,
    type PlaylistSettings,
    type Resource,
    type SourceDetail,
} from '../personal';
import { EmptyState, Heading, Icon } from '../components/common';
import { ContinuousList } from '../components/ContinuousList';
import { ChannelPicker } from './PersonalGuide';
import type { Channel } from '../types';

const roles: [Resource, string][] = [
    ['playlist', 'Download della lista'],
    ['manifest', 'Manifest'],
    ['media', 'Video e audio'],
    ['license', 'Licenze DRM'],
    ['key', 'Chiavi HLS'],
    ['logo', 'Loghi'],
    ['guide', 'Guida XMLTV'],
];
const sections = [
    'Sorgente',
    'Lettura e contenuti',
    'Accesso ai server',
    'Guida e archivio',
    'Stato e ultima importazione',
];
const lines = (value: string) =>
    value
        .split('\n')
        .map((s) => s.trim())
        .filter(Boolean);
function Field({ label, children, help }: { label: string; children: ReactNode; help?: string }) {
    const id = useId();
    return (
        <label className="library-field">
            <span id={id}>{label}</span>
            {isValidElement(children)
                ? cloneElement(children as ReactElement<Record<string, unknown>>, {
                      'aria-labelledby': id,
                      'aria-describedby': help ? `${id}-help` : undefined,
                  })
                : children}
            {help && <small id={`${id}-help`}>{help}</small>}
        </label>
    );
}
function Check({
    label,
    checked,
    onChange,
    help,
}: {
    label: string;
    checked: boolean;
    onChange(value: boolean): void;
    help?: string;
}) {
    return (
        <label className="library-check">
            <input
                type="checkbox"
                aria-label={label}
                checked={checked}
                onChange={(e) => onChange(e.target.checked)}
            />
            <span>
                {label}
                {help && <small>{help}</small>}
            </span>
        </label>
    );
}
function NumberField({
    label,
    value,
    min = 0,
    max,
    onChange,
    help,
}: {
    label: string;
    value: number;
    min?: number;
    max: number;
    onChange(value: number): void;
    help?: string;
}) {
    return (
        <Field label={label} help={help}>
            <input
                type="number"
                min={min}
                max={max}
                value={value}
                onChange={(e) => onChange(Number(e.target.value))}
            />
        </Field>
    );
}
function GroupContentEditor({
    settings,
    onChange,
}: {
    settings: PlaylistSettings;
    onChange(value: PlaylistSettings): void;
}) {
    const [name, setName] = useState('');
    const rules = settings.group_content || {};
    return (
        <section className="mapping-editor">
            <h3>Contenuti per gruppo</h3>
            <p>
                Forza un gruppo come dirette o video. Le regole precedono il riconoscimento
                automatico; in caso di conflitto prevalgono le dirette.
            </p>
            {Object.entries(rules).map(([group, kind]) => (
                <div className="library-rule" key={group}>
                    <span>{group}</span>
                    <select
                        aria-label={`Contenuto di ${group}`}
                        value={kind}
                        onChange={(e) =>
                            onChange({
                                ...settings,
                                group_content: { ...rules, [group]: e.target.value },
                            })
                        }
                    >
                        <option value="live">Dirette</option>
                        <option value="vod">Video</option>
                    </select>
                    <button
                        type="button"
                        className="text-button"
                        aria-label={`Rimuovi regola ${group}`}
                        onClick={() => {
                            const next = { ...rules };
                            delete next[group];
                            onChange({ ...settings, group_content: next });
                        }}
                    >
                        Rimuovi
                    </button>
                </div>
            ))}
            <div className="library-rule">
                <input
                    aria-label="Nome del gruppo"
                    maxLength={100}
                    value={name}
                    placeholder="Nome esatto del gruppo"
                    onChange={(e) => setName(e.target.value)}
                />
                <button
                    type="button"
                    className="button secondary"
                    disabled={!name.trim() || Object.keys(rules).length >= 100}
                    onClick={() => {
                        onChange({
                            ...settings,
                            group_content: { ...rules, [name.trim()]: 'vod' },
                        });
                        setName('');
                    }}
                >
                    Aggiungi regola
                </button>
            </div>
        </section>
    );
}
function MappingEditor({
    source,
    settings,
    onChange,
}: {
    source: string;
    settings: PlaylistSettings;
    onChange(settings: PlaylistSettings): void;
}) {
    const [channel, select] = useState<Channel | null>(null);
    const [query, setQuery] = useState('');
    const mapping = channel ? settings.guide.mappings[channel.id] : undefined;
    function change(value?: string) {
        if (!channel) return;
        const mappings = { ...settings.guide.mappings };
        if (value === undefined) delete mappings[channel.id];
        else mappings[channel.id] = value;
        onChange({ ...settings, guide: { ...settings.guide, mappings } });
    }
    return (
        <section className="mapping-editor">
            <h3>Associa canali</h3>
            <p>Scegli un canale della lista e il corrispondente nella guida scaricata.</p>
            <div className="mapping-columns">
                <ChannelPicker
                    source={source}
                    selected={channel?.id}
                    onSelect={select}
                    height={260}
                />
                <section className="library-picker" aria-label="Associazione nella guida">
                    {channel ? (
                        <>
                            <h4>{channel.name}</h4>
                            <div className="library-actions">
                                <button
                                    className="text-button"
                                    aria-pressed={mapping === undefined}
                                    onClick={() => change()}
                                >
                                    Automatica
                                </button>
                                <button
                                    className="text-button"
                                    aria-pressed={mapping === ''}
                                    onClick={() => change('')}
                                >
                                    Escludi dalla guida
                                </button>
                            </div>
                            <Field label="Cerca nella guida">
                                <input
                                    type="search"
                                    value={query}
                                    onChange={(e) => setQuery(e.target.value)}
                                />
                            </Field>
                            <ContinuousList<{
                                key: string;
                                name: string;
                                xml_id: string;
                                feed: number;
                            }>
                                context={`mapping:${source}:${query}`}
                                label="Canali XMLTV"
                                height={220}
                                rowHeight={56}
                                blockSize={50}
                                load={(offset, signal) =>
                                    api<
                                        LibraryPage<{
                                            key: string;
                                            name: string;
                                            xml_id: string;
                                            feed: number;
                                        }>
                                    >(
                                        '/api/library/guide/choices',
                                        { id: source, query, offset },
                                        { signal },
                                    )
                                }
                                render={(choice) => (
                                    <button
                                        className="library-choice"
                                        aria-pressed={mapping === choice.key}
                                        onClick={() => change(choice.key)}
                                    >
                                        <strong>{choice.name}</strong>
                                        <small>
                                            {choice.xml_id} · Guida {choice.feed + 1}
                                        </small>
                                    </button>
                                )}
                                empty={
                                    <p className="notice">
                                        Nessun canale trovato. Verifica o aggiorna la guida della
                                        lista.
                                    </p>
                                }
                            />
                        </>
                    ) : (
                        <p className="notice">Scegli un canale a sinistra.</p>
                    )}
                </section>
            </div>
        </section>
    );
}

function SourceForm({
    initial,
    importing,
    replacement,
    onBack,
    onReload,
}: {
    initial: SourceDetail;
    importing: boolean;
    replacement: boolean;
    onBack(): void;
    onReload(id: string): void;
}) {
    const state = useStore(library);
    const current = state.sources.find((source) => source.id === initial.id);
    const [section, setSection] = useState(0);
    const [name, setName] = useState(initial.name);
    const [url, setUrl] = useState(initial.url || '');
    const [location, setLocation] = useState<'url' | 'file'>(
        initial.url || !initial.id ? 'url' : 'file',
    );
    const [file, setFile] = useState<File | null>(null);
    const [enabled, setEnabled] = useState(initial.enabled);
    const [settings, setSettings] = useState(initial.settings);
    const [role, setRole] = useState<Resource>('playlist');
    const [headers, setHeaders] = useState<Record<string, string>>(() =>
        Object.fromEntries(
            roles.map(([role]) => [
                role,
                Object.entries(initial.settings.access[role]?.headers || {})
                    .map(([k, v]) => `${k}: ${v}`)
                    .join('\n'),
            ]),
        ),
    );
    const [origins, setOrigins] = useState<Record<string, string>>(() =>
        Object.fromEntries(
            roles.map(([role]) => [
                role,
                (initial.settings.access[role]?.origins || []).join('\n'),
            ]),
        ),
    );
    const [job, setJob] = useState<ImportJob | null>(null);
    const [error, setError] = useState('');
    const [notice, setNotice] = useState('');
    const [saving, setSaving] = useState(false);
    const [back, setBack] = useState(false);
    const [deleting, setDeleting] = useState(false);
    const [mapping, setMapping] = useState(false);
    const upload = useRef<AbortController | null>(null);
    const prepared = useRef('');
    const snapshot = JSON.stringify({
        name,
        url,
        enabled,
        settings,
        headers,
        origins,
        file: file && [file.name, file.size, file.lastModified],
        location,
    });
    const baseline = useRef(snapshot);
    const dirty = snapshot !== baseline.current;
    const active = !!job && !['ready', 'done', 'error', 'cancelled'].includes(job.phase);
    const blocked = saving || active;
    useEffect(() => {
        if (!job || ['ready', 'done', 'error', 'cancelled', 'upload'].includes(job.phase)) return;
        const timer = setTimeout(
            () =>
                void api<ImportJob>('/api/library/job', { id: job.id })
                    .then(setJob)
                    .catch((e) => setError(errorMessage(e))),
            700,
        );
        return () => clearTimeout(timer);
    }, [job]);
    useEffect(() => {
        if (job?.phase === 'done') {
            void refreshLibrary();
            baseline.current = snapshot;
            setNotice('Lista salvata. I contenuti sono disponibili nel catalogo.');
        }
        if (job?.phase === 'error') setError(job.error);
    }, [job?.phase]);
    useEffect(() => {
        if (job?.phase === 'ready' && prepared.current !== snapshot) {
            void api('/api/library/cancel', { id: job.id });
            setJob(null);
            setNotice('Le opzioni sono cambiate. Verifica di nuovo la lista.');
        }
    }, [snapshot, job]);
    useEffect(
        () => () => {
            upload.current?.abort();
        },
        [],
    );
    function finalSettings() {
        const access: Partial<Record<Resource, Access>> = {};
        for (const [role] of roles) {
            const values: Record<string, string> = {};
            for (const line of lines(headers[role] || '')) {
                const colon = line.indexOf(':');
                if (colon < 1) throw { code: 'Scrivi ogni header nella forma Nome: valore.' };
                values[line.slice(0, colon).trim()] = line.slice(colon + 1).trim();
            }
            const allowed = lines(origins[role] || '');
            if (Object.keys(values).length || allowed.length)
                access[role] = { headers: values, origins: allowed };
        }
        return {
            ...settings,
            access,
            guide: { ...settings.guide, urls: lines(settings.guide.urls.join('\n')) },
        };
    }
    function guide(patch: Partial<PlaylistSettings['guide']>) {
        setSettings({ ...settings, guide: { ...settings.guide, ...patch } });
    }
    async function verify() {
        setError('');
        setNotice('');
        try {
            if (!name.trim()) throw { code: 'Inserisci un nome per la lista.' };
            if (location === 'file' && !file) throw { code: 'Scegli un file M3U o M3U8.' };
            if (file && file.size > 256 * 1024 * 1024) throw { code: 'Il file supera 256 MiB.' };
            if (location === 'url' && !url.trim())
                throw { code: 'Inserisci l’indirizzo della lista.' };
            const next = await api<ImportJob>('/api/library/prepare', {
                source_id: initial.id,
                revision: initial.revision,
                name,
                url: location === 'url' ? url.trim() : null,
                settings: finalSettings(),
            });
            prepared.current = snapshot;
            setJob(next);
            if (location === 'file' && file) {
                setJob({ ...next, phase: 'uploading' });
                const controller = new AbortController();
                upload.current = controller;
                const response = await apiFetch(`/api/library/upload/${next.id}`, {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/octet-stream' },
                    body: file,
                    signal: controller.signal,
                });
                const result = await response.json();
                if (!response.ok) throw { code: result.error };
                setJob(result);
            }
        } catch (e) {
            if ((e as { name?: string }).name !== 'AbortError') setError(errorMessage(e));
        }
    }
    async function cancel() {
        if (!job) return;
        try {
            setJob(await api<ImportJob>('/api/library/cancel', { id: job.id }));
            upload.current?.abort();
        } catch (e) {
            setError(errorMessage(e));
        }
    }
    async function save() {
        setError('');
        setSaving(true);
        try {
            await api('/api/library/edit', {
                id: initial.id,
                revision: initial.revision,
                name,
                enabled,
                settings: finalSettings(),
            });
            if (
                playback.store.get().channel?.source_id === initial.id &&
                (!enabled || settings.allow_http !== initial.settings.allow_http)
            )
                await playback.stop();
            await refreshLibrary();
            onReload(initial.id);
        } catch (e) {
            setError(errorMessage(e));
        } finally {
            setSaving(false);
        }
    }
    async function remove() {
        setSaving(true);
        setError('');
        try {
            await api('/api/library/delete', { id: initial.id, revision: initial.revision });
            if (playback.store.get().channel?.source_id === initial.id) await playback.stop();
            await refreshLibrary();
            onBack();
        } catch (e) {
            setError(errorMessage(e));
        } finally {
            setSaving(false);
        }
    }
    async function refresh() {
        setError('');
        try {
            setJob(await api<ImportJob>('/api/library/refresh', { id: initial.id }));
        } catch (e) {
            setError(errorMessage(e));
        }
    }
    const report = job?.report || current?.report;
    const progress =
        job &&
        ({
            upload: 'Scegli il file',
            uploading: 'Caricamento del file',
            reading: 'Lettura e verifica della lista',
            indexing: 'Organizzazione dei canali',
            preparing: 'Organizzazione dei canali',
            saving: 'Salvataggio finale',
            ready: 'Anteprima pronta',
            done: 'Lista salvata',
            cancelling: 'Annullamento in corso',
            cancelled: 'Importazione annullata',
            error: 'Verifica interrotta',
        }[job.phase] ||
            'Verifica in corso');
    return (
        <>
            <div className="library-breadcrumb">
                <button
                    className="text-button"
                    disabled={blocked}
                    onClick={() =>
                        dirty || active || job?.phase === 'ready' ? setBack(true) : onBack()
                    }
                >
                    <Icon name="chevron" className="reverse-chevron" />
                    Le mie liste
                </button>
            </div>
            <Heading
                title={
                    importing
                        ? replacement
                            ? 'Sostituisci la sorgente.'
                            : 'Aggiungi una lista.'
                        : name || 'La tua lista.'
                }
                subtitle={
                    importing
                        ? 'Verifica i contenuti e controlla l’anteprima prima di importarli.'
                        : 'Sorgente, accesso, guida e contenuti della lista.'
                }
            />
            {back && (
                <div className="library-confirm" role="region" aria-label="Modifiche non salvate">
                    <p>Hai modifiche o un’anteprima da salvare.</p>
                    <button className="button secondary" autoFocus onClick={() => setBack(false)}>
                        Continua a modificare
                    </button>
                    <button
                        className="text-button"
                        onClick={async () => {
                            await cancel();
                            onBack();
                        }}
                    >
                        Scarta modifiche
                    </button>
                </div>
            )}
            {error && (
                <p className="library-error" role="alert">
                    {error}
                </p>
            )}
            {notice && (
                <p className="notice" role="status">
                    {notice}
                </p>
            )}
            <div className="library-editor">
                <nav className="library-sections" aria-label="Sezioni della lista">
                    {sections.map((label, index) => (
                        <button
                            key={label}
                            aria-current={section === index ? 'page' : undefined}
                            onClick={() => setSection(index)}
                        >
                            {label}
                            <Icon name="chevron" />
                        </button>
                    ))}
                </nav>
                <div className="library-editor-content">
                    <fieldset disabled={blocked || job?.phase === 'done'}>
                        <h2>{sections[section]}</h2>
                        {section === 0 && (
                            <>
                                <Field label="Nome della lista">
                                    <input
                                        type="text"
                                        maxLength={160}
                                        value={name}
                                        onChange={(e) => setName(e.target.value)}
                                    />
                                </Field>
                                {importing ? (
                                    <>
                                        <div className="segmented" aria-label="Origine della lista">
                                            <button
                                                aria-pressed={location === 'url'}
                                                onClick={() => setLocation('url')}
                                            >
                                                Da indirizzo
                                            </button>
                                            <button
                                                aria-pressed={location === 'file'}
                                                onClick={() => setLocation('file')}
                                            >
                                                Da file
                                            </button>
                                        </div>
                                        {location === 'url' ? (
                                            <Field
                                                label="Indirizzo della lista"
                                                help="HTTP o HTTPS. Gli indirizzi e i dati di accesso restano sul computer."
                                            >
                                                <input
                                                    type="url"
                                                    autoComplete="off"
                                                    spellCheck={false}
                                                    value={url}
                                                    onChange={(e) => setUrl(e.target.value)}
                                                />
                                            </Field>
                                        ) : (
                                            <Field
                                                label="File della lista"
                                                help="M3U o M3U8, fino a 256 MiB. Verrà conservata una copia privata dei contenuti."
                                            >
                                                <input
                                                    type="file"
                                                    accept=".m3u,.m3u8,text/plain,audio/x-mpegurl"
                                                    onChange={(e) =>
                                                        setFile(e.target.files?.[0] || null)
                                                    }
                                                />
                                            </Field>
                                        )}
                                    </>
                                ) : (
                                    <>
                                        <p className="library-address">
                                            {initial.url || 'Lista importata da file'}
                                        </p>
                                        <Check
                                            label="Lista attiva"
                                            checked={enabled}
                                            onChange={setEnabled}
                                            help="Le liste disattivate conservano dati e preferiti."
                                        />
                                    </>
                                )}
                                <Check
                                    label="Consenti HTTP per questa lista"
                                    checked={settings.allow_http}
                                    onChange={(allow_http) =>
                                        setSettings({ ...settings, allow_http })
                                    }
                                    help="Vale per download, video e licenze della sorgente. HTTP trasmette senza cifratura gli indirizzi e gli eventuali dati di accesso."
                                />
                                {!importing && (
                                    <div className="library-actions">
                                        <button
                                            className="button secondary"
                                            disabled={
                                                dirty ||
                                                current?.file ||
                                                (current?.retry_at || 0) > Date.now()
                                            }
                                            onClick={() => void refresh()}
                                        >
                                            Aggiorna lista
                                        </button>
                                        <button
                                            className="text-button"
                                            disabled={dirty}
                                            onClick={() => onReload(`replace:${initial.id}`)}
                                        >
                                            Sostituisci sorgente
                                        </button>
                                    </div>
                                )}
                                {!importing && (
                                    <div className="library-delete">
                                        <button
                                            className="text-button"
                                            onClick={() => setDeleting(true)}
                                        >
                                            Elimina lista
                                        </button>
                                        {deleting && (
                                            <div
                                                className="library-confirm"
                                                role="region"
                                                aria-label="Elimina lista"
                                            >
                                                <p>
                                                    Eliminare questa lista, la sua guida e gli
                                                    avanzamenti dei video?
                                                </p>
                                                <button
                                                    className="button secondary"
                                                    autoFocus
                                                    onClick={() => setDeleting(false)}
                                                >
                                                    Annulla
                                                </button>
                                                <button
                                                    className="button secondary"
                                                    onClick={() => void remove()}
                                                >
                                                    Elimina lista e dati
                                                </button>
                                            </div>
                                        )}
                                    </div>
                                )}
                            </>
                        )}
                        {section === 1 && (
                            <>
                                {!importing && (
                                    <p>
                                        Se cambi tipo, formato, codifica o gruppi, aggiorna la lista
                                        o sostituisci il file per rileggerla. La scelta tra dirette
                                        e video si applica subito.
                                    </p>
                                )}
                                <Field label="Tipo di lista">
                                    <select
                                        value={settings.kind}
                                        onChange={(e) =>
                                            setSettings({
                                                ...settings,
                                                kind: e.target.value as 'tv' | 'radio',
                                            })
                                        }
                                    >
                                        <option value="tv">Canali TV e video</option>
                                        <option value="radio">Radio</option>
                                    </select>
                                </Field>
                                <Field label="Formato">
                                    <select
                                        value={settings.format}
                                        onChange={(e) =>
                                            setSettings({ ...settings, format: e.target.value })
                                        }
                                    >
                                        <option value="extended">M3U estesa</option>
                                        <option value="simple">M3U semplice</option>
                                    </select>
                                </Field>
                                <Field label="Codifica">
                                    <select
                                        value={settings.encoding}
                                        onChange={(e) =>
                                            setSettings({ ...settings, encoding: e.target.value })
                                        }
                                    >
                                        <option value="utf-8">UTF-8</option>
                                        <option value="windows-1252">Windows-1252</option>
                                        <option value="latin-1">Latin-1</option>
                                    </select>
                                </Field>
                                <Field label="Gruppi">
                                    <select
                                        value={settings.group_profile}
                                        onChange={(e) =>
                                            setSettings({
                                                ...settings,
                                                group_profile: e.target.value,
                                            })
                                        }
                                    >
                                        <option value="default">Predefinito</option>
                                        <option value="kodi">Compatibilità Kodi</option>
                                        <option value="ott">Compatibilità OTT Navigator</option>
                                    </select>
                                </Field>
                                <Field label="Tipo dei contenuti">
                                    <select
                                        value={settings.content}
                                        onChange={(e) =>
                                            setSettings({ ...settings, content: e.target.value })
                                        }
                                    >
                                        <option value="auto">Riconoscimento automatico</option>
                                        <option value="live">Solo dirette</option>
                                        <option value="vod">Solo video</option>
                                    </select>
                                </Field>
                                <Field
                                    label="Riconoscimento dei video"
                                    help="Prudente segue le informazioni dichiarate dalla lista. Esteso riconosce anche file video nei gruppi Film, Serie o VOD; esclude dirette e archivio."
                                >
                                    <select
                                        value={settings.vod_detection || 'prudent'}
                                        onChange={(e) =>
                                            setSettings({
                                                ...settings,
                                                vod_detection: e.target.value,
                                            })
                                        }
                                    >
                                        <option value="prudent">Prudente</option>
                                        <option value="extended">Esteso</option>
                                    </select>
                                </Field>
                                <GroupContentEditor settings={settings} onChange={setSettings} />
                                <NumberField
                                    label="Aggiornamento automatico (minuti)"
                                    value={settings.refresh_minutes}
                                    max={10080}
                                    onChange={(refresh_minutes) =>
                                        setSettings({ ...settings, refresh_minutes })
                                    }
                                    help="0 segue l’intervallo della lista o usa 24 ore; altrimenti da 15 minuti a 7 giorni. Gli aggiornamenti avvengono mentre Lume è aperta."
                                />
                            </>
                        )}
                        {section === 2 && (
                            <>
                                <Field label="Funzione">
                                    <select
                                        value={role}
                                        onChange={(e) => setRole(e.target.value as Resource)}
                                    >
                                        {roles.map(([value, label]) => (
                                            <option key={value} value={value}>
                                                {label}
                                            </option>
                                        ))}
                                    </select>
                                </Field>
                                <Field
                                    label="Header HTTP"
                                    help="Uno per riga, nella forma Nome: valore. Questi valori prevalgono su quelli della lista."
                                >
                                    <textarea
                                        rows={5}
                                        autoComplete="off"
                                        spellCheck={false}
                                        value={headers[role]}
                                        onChange={(e) =>
                                            setHeaders({ ...headers, [role]: e.target.value })
                                        }
                                    />
                                </Field>
                                <Field
                                    label="Altri server autorizzati"
                                    help="Un’origine per riga, con protocollo, host e l’eventuale porta. Massimo 16 per funzione. I dati di accesso vengono rimossi nei passaggi verso altri server."
                                >
                                    <textarea
                                        rows={3}
                                        autoComplete="off"
                                        spellCheck={false}
                                        value={origins[role]}
                                        onChange={(e) =>
                                            setOrigins({ ...origins, [role]: e.target.value })
                                        }
                                    />
                                </Field>
                                <Check
                                    label="Mantieni cookie di sessione"
                                    checked={settings.cookies}
                                    onChange={(cookies) => setSettings({ ...settings, cookies })}
                                    help="Cookie separati per lista e funzione, mantenuti soltanto durante la sessione dell’app."
                                />
                                <Check
                                    label="Consenti HTTP per i loghi"
                                    checked={settings.allow_http_logos}
                                    onChange={(allow_http_logos) =>
                                        setSettings({ ...settings, allow_http_logos })
                                    }
                                    help="Consenso separato da quello per video e download."
                                />
                            </>
                        )}
                        {section === 3 && (
                            <>
                                <Check
                                    label="Guida XMLTV personale attiva"
                                    checked={settings.guide.enabled}
                                    onChange={(enabled) => guide({ enabled })}
                                />
                                <Field
                                    label="Indirizzi della guida"
                                    help="Uno per riga, fino a 16. XML, gzip o xz. Se lasci vuoto, Lume usa la guida dichiarata dalla lista. Quando manca, cerca prima nella guida Lume e poi in EPGShare IT1, solo per i canali ancora senza programmi."
                                >
                                    <textarea
                                        rows={3}
                                        autoComplete="off"
                                        value={settings.guide.urls.join('\n')}
                                        onChange={(e) =>
                                            guide({ urls: e.target.value.split('\n') })
                                        }
                                        onBlur={() =>
                                            guide({ urls: lines(settings.guide.urls.join('\n')) })
                                        }
                                    />
                                </Field>
                                <Check
                                    label="Consenti HTTP per questa guida"
                                    checked={settings.guide.allow_http}
                                    onChange={(allow_http) => guide({ allow_http })}
                                />
                                <div className="library-field-grid">
                                    <Field label="Lingua preferita">
                                        <input
                                            value={settings.guide.language}
                                            maxLength={20}
                                            onChange={(e) => guide({ language: e.target.value })}
                                        />
                                    </Field>
                                    <Field label="Fuso degli orari senza offset">
                                        <input
                                            value={settings.guide.timezone}
                                            onChange={(e) => guide({ timezone: e.target.value })}
                                        />
                                    </Field>
                                    <NumberField
                                        label="Correzione della guida (minuti)"
                                        min={-1440}
                                        max={1440}
                                        value={settings.guide.shift_minutes}
                                        onChange={(shift_minutes) => guide({ shift_minutes })}
                                    />
                                    <NumberField
                                        label="Giorni precedenti da conservare"
                                        min={1}
                                        max={7}
                                        value={settings.guide.history_days}
                                        onChange={(history_days) => guide({ history_days })}
                                    />
                                    <NumberField
                                        label="Ore successive da conservare"
                                        min={1}
                                        max={48}
                                        value={settings.guide.future_hours}
                                        onChange={(future_hours) => guide({ future_hours })}
                                    />
                                </div>
                                {!importing && (
                                    <div className="library-actions">
                                        <button
                                            className="button secondary"
                                            disabled={
                                                dirty ||
                                                (!settings.guide.enabled &&
                                                    current?.guide.origin !== 'lume')
                                            }
                                            onClick={async () => {
                                                await api('/api/library/guide/refresh', {
                                                    id: initial.id,
                                                }).catch((e) => setError(errorMessage(e)));
                                                await refreshLibrary();
                                            }}
                                        >
                                            Aggiorna guida
                                        </button>
                                        {current?.guide.origin !== 'lume' && (
                                            <button
                                                className="text-button"
                                                onClick={() => setMapping(!mapping)}
                                            >
                                                Associa canali
                                            </button>
                                        )}
                                        {current?.guide.origin !== 'lume' && (
                                            <button
                                                className="text-button"
                                                disabled={dirty}
                                                onClick={async () => {
                                                    await api('/api/library/guide/clear', {
                                                        id: initial.id,
                                                    }).catch((e) => setError(errorMessage(e)));
                                                    await refreshLibrary();
                                                    onReload(initial.id);
                                                }}
                                            >
                                                Svuota guida scaricata
                                            </button>
                                        )}
                                    </div>
                                )}
                                {mapping && !importing && current?.guide.origin !== 'lume' && (
                                    <MappingEditor
                                        source={initial.id}
                                        settings={settings}
                                        onChange={setSettings}
                                    />
                                )}
                                <h3>Archivio e dall’inizio</h3>
                                <p>
                                    Disponibili quando la lista e il servizio dichiarano un archivio
                                    compatibile. Non vengono create registrazioni locali.
                                </p>
                                <div className="library-field-grid">
                                    <Field
                                        label="Disponibilità dell’archivio (giorni)"
                                        help="Vuoto usa la disponibilità dichiarata dal canale."
                                    >
                                        <input
                                            type="number"
                                            min={0}
                                            max={365}
                                            value={settings.guide.archive_days ?? ''}
                                            onChange={(e) =>
                                                guide({
                                                    archive_days:
                                                        e.target.value === ''
                                                            ? null
                                                            : Number(e.target.value),
                                                })
                                            }
                                        />
                                    </Field>
                                    <Field label="Fuso del servizio archivio">
                                        <input
                                            value={settings.guide.archive_timezone}
                                            onChange={(e) =>
                                                guide({ archive_timezone: e.target.value })
                                            }
                                        />
                                    </Field>
                                    <NumberField
                                        label="Margine iniziale (secondi)"
                                        value={settings.guide.before_seconds}
                                        max={300}
                                        onChange={(before_seconds) => guide({ before_seconds })}
                                    />
                                    <NumberField
                                        label="Margine finale (secondi)"
                                        value={settings.guide.after_seconds}
                                        max={300}
                                        onChange={(after_seconds) => guide({ after_seconds })}
                                    />
                                </div>
                                <Check
                                    label="Consenti dall’inizio durante un programma in onda"
                                    checked={settings.guide.ongoing}
                                    onChange={(ongoing) => guide({ ongoing })}
                                />
                            </>
                        )}
                        {section === 4 && (
                            <>
                                {current && (
                                    <dl className="library-status">
                                        <dt>Ultima importazione</dt>
                                        <dd>
                                            {date(current.updated_at)}, {time(current.updated_at)}
                                        </dd>
                                        <dt>Guida</dt>
                                        <dd>
                                            {current.guide.loading
                                                ? 'Aggiornamento in corso'
                                                : current.guide.origin === 'lume'
                                                  ? `Guida Lume · ${(current.guide.channel_count || 0).toLocaleString('it-IT')} canali nel feed completo`
                                                  : `${current.guide.matched} canali associati · ${current.guide.programmes} programmi`}
                                        </dd>
                                        {current.guide.origin === 'lume' &&
                                            current.guide.supplement && (
                                                <>
                                                    <dt>Integrazione EPGShare IT1</dt>
                                                    <dd>
                                                        {current.guide.supplement.loading
                                                            ? 'Aggiornamento in corso'
                                                            : `${current.guide.supplement.channel_count.toLocaleString('it-IT')} canali nella guida integrativa`}
                                                    </dd>
                                                </>
                                            )}
                                    </dl>
                                )}
                                {current?.error && <p className="library-error">{current.error}</p>}
                                {current?.guide.error && (
                                    <p className="library-error">{current.guide.error}</p>
                                )}
                                {report ? (
                                    <>
                                        <p>
                                            {report.count.toLocaleString('it-IT')} voci importabili
                                            · {report.vod.toLocaleString('it-IT')} video ·{' '}
                                            {report.radio.toLocaleString('it-IT')} radio
                                        </p>
                                        <p>
                                            {report.excluded} escluse · {report.duplicates}{' '}
                                            duplicate
                                        </p>
                                        {Object.entries(report.diagnostics).map(
                                            ([reason, count]) => (
                                                <p key={reason} className="notice">
                                                    {count.toLocaleString('it-IT')} · {reason}
                                                </p>
                                            ),
                                        )}
                                    </>
                                ) : (
                                    <p className="notice">
                                        Verifica una lista per vedere conteggi e motivi degli
                                        scarti.
                                    </p>
                                )}
                                <p className="notice">
                                    Fino a 500.000 voci per sorgente e un milione complessive, con
                                    dieci liste. URL, header e chiavi restano nell’archivio privato
                                    del computer.
                                </p>
                            </>
                        )}
                    </fieldset>
                    <div className="library-save">
                        <button
                            className="button primary"
                            disabled={
                                blocked ||
                                job?.phase === 'ready' ||
                                job?.phase === 'done' ||
                                (!importing && !dirty)
                            }
                            onClick={() => void (importing ? verify() : save())}
                        >
                            {importing
                                ? 'Verifica lista'
                                : saving
                                  ? 'Salvataggio…'
                                  : 'Salva modifiche'}
                        </button>
                    </div>
                    {job && (
                        <section
                            className="import-progress"
                            aria-label="Importazione della lista"
                            aria-busy={active}
                        >
                            <h3>{progress}</h3>
                            <p role="status">
                                {(job.bytes / 1024 / 1024).toLocaleString('it-IT', {
                                    maximumFractionDigits: 1,
                                })}{' '}
                                MiB · {job.entries.toLocaleString('it-IT')} voci lette
                            </p>
                            {active && <progress aria-label="Verifica in corso" />}
                            {job.phase === 'ready' && job.report && (
                                <>
                                    <p>
                                        {job.report.count.toLocaleString('it-IT')} voci importabili
                                        · {job.report.excluded} escluse · {job.report.duplicates}{' '}
                                        duplicate
                                    </p>
                                    <ul>
                                        {job.report.sample.map((name, i) => (
                                            <li key={i}>{name}</li>
                                        ))}
                                    </ul>
                                    <button
                                        className="button primary"
                                        onClick={async () => {
                                            try {
                                                setJob(
                                                    await api<ImportJob>('/api/library/commit', {
                                                        id: job.id,
                                                    }),
                                                );
                                            } catch (e) {
                                                setError(errorMessage(e));
                                            }
                                        }}
                                    >
                                        {replacement ? 'Sostituisci lista' : 'Importa lista'}
                                    </button>
                                </>
                            )}
                            {(active || job.phase === 'ready') && job.phase !== 'saving' && (
                                <button className="text-button" onClick={() => void cancel()}>
                                    Interrompi verifica
                                </button>
                            )}
                            {job.phase === 'done' && (
                                <div className="library-actions">
                                    <button
                                        className="button primary"
                                        onClick={() => {
                                            const radio =
                                                !!job.report?.count &&
                                                job.report.radio === job.report.count;
                                            (radio ? personalRadioSelection : librarySelection).set(
                                                { source: job.source_id, query: '', group: '' },
                                            );
                                            navigate(radio ? 'radio' : 'channels');
                                        }}
                                    >
                                        Apri il catalogo
                                    </button>
                                    <button
                                        className="text-button"
                                        onClick={() => onReload(job.source_id)}
                                    >
                                        Gestisci lista
                                    </button>
                                </div>
                            )}
                        </section>
                    )}
                </div>
            </div>
        </>
    );
}

export function Playlists() {
    const state = useStore(library);
    const [editor, setEditor] = useState<{
        detail: SourceDetail;
        importing: boolean;
        replacement: boolean;
    } | null>(null);
    const [error, setError] = useState('');
    const [loading, setLoading] = useState(false);
    async function open(id: string) {
        const replacement = id.startsWith('replace:');
        id = id.replace(/^replace:/, '');
        setError('');
        setLoading(true);
        try {
            setEditor({
                detail: await api<SourceDetail>('/api/library/source', { id }),
                importing: replacement,
                replacement,
            });
        } catch (e) {
            setError(errorMessage(e));
        } finally {
            setLoading(false);
        }
    }
    useEffect(() => {
        void refreshLibrary();
    }, []);
    useEffect(() => {
        if (!state.imports.length) return;
        const timer = setInterval(() => void refreshLibrary(), 1000);
        return () => clearInterval(timer);
    }, [state.imports.length]);
    if (editor)
        return (
            <SourceForm
                key={`${editor.detail.id}:${editor.detail.revision}:${editor.importing}`}
                initial={editor.detail}
                importing={editor.importing}
                replacement={editor.replacement}
                onBack={() => setEditor(null)}
                onReload={(id) => void open(id)}
            />
        );
    return (
        <>
            <div className="library-breadcrumb">
                <button className="text-button" onClick={() => navigate('settings')}>
                    <Icon name="chevron" className="reverse-chevron" />
                    Impostazioni
                </button>
            </div>
            <Heading
                title="Le mie liste."
                subtitle="Aggiungi i tuoi canali e video. Ogni lista mantiene accesso, guida e impostazioni separati."
            />
            <div className="library-actions">
                <button
                    className="button primary"
                    disabled={state.sources.length >= 10 || loading}
                    onClick={() =>
                        setEditor({
                            detail: {
                                id: '',
                                name: '',
                                enabled: true,
                                revision: 0,
                                url: null,
                                settings: defaultSettings(),
                            },
                            importing: true,
                            replacement: false,
                        })
                    }
                >
                    <Icon name="grid" />
                    Aggiungi lista
                </button>
                <span className="notice">{state.sources.length} di 10 liste</span>
            </div>
            {(error || state.error) && (
                <p className="library-error" role="alert">
                    {error || state.error}
                </p>
            )}
            {state.imports.map((job) => (
                <section className="import-progress" key={job.id}>
                    <h2>{job.name || 'Importazione in corso'}</h2>
                    <p>
                        {job.phase === 'ready' ? 'Anteprima pronta' : 'Verifica in corso'} ·{' '}
                        {job.entries.toLocaleString('it-IT')} voci lette
                    </p>
                    {job.phase === 'ready' && job.report && (
                        <>
                            <p>
                                {job.report.count.toLocaleString('it-IT')} voci importabili ·{' '}
                                {job.report.excluded} escluse
                            </p>
                            <ul>
                                {job.report.sample.map((name, i) => (
                                    <li key={i}>{name}</li>
                                ))}
                            </ul>
                            <button
                                className="button primary"
                                onClick={async () => {
                                    await api('/api/library/commit', { id: job.id }).catch((e) =>
                                        setError(errorMessage(e)),
                                    );
                                    await refreshLibrary();
                                }}
                            >
                                Importa lista
                            </button>
                        </>
                    )}
                    <button
                        className="text-button"
                        disabled={job.phase === 'saving'}
                        onClick={async () => {
                            await api('/api/library/cancel', { id: job.id }).catch((e) =>
                                setError(errorMessage(e)),
                            );
                            await refreshLibrary();
                        }}
                    >
                        Interrompi verifica
                    </button>
                </section>
            ))}
            {loading && (
                <p className="notice" role="status">
                    Apertura della lista…
                </p>
            )}
            {state.sources.length ? (
                <div className="personal-sources">
                    {state.sources.map((source) => (
                        <button
                            className="personal-source"
                            key={source.id}
                            onClick={() => void open(source.id)}
                            disabled={loading}
                        >
                            <span>
                                <strong>{source.name}</strong>
                                <small>
                                    {source.enabled
                                        ? `${source.report.count.toLocaleString('it-IT')} voci · ${source.file ? 'Da file' : 'Da indirizzo'}`
                                        : 'Disattivata'}
                                    {source.error ? ' · Da verificare' : ''}
                                </small>
                            </span>
                            <Icon name="chevron" />
                        </button>
                    ))}
                </div>
            ) : (
                state.ready && (
                    <EmptyState
                        symbol="grid"
                        title="Porta qui le tue liste."
                        description="Importa una lista M3U da un indirizzo o da un file. Potrai verificarla prima di salvarla."
                    />
                )
            )}
        </>
    );
}
