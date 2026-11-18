import { useEffect, useRef, useState } from 'react';
import { api } from '../api';
import { play, savePreferences } from '../app';
import {
    personalCatalog,
    readSession,
    writeSession,
    type LibraryPage,
    type PersonalProgramme,
    type PersonalSource,
} from '../personal';
import { ContinuousList } from '../components/ContinuousList';
import { range } from '../domain';
import type { Channel } from '../types';

export function ChannelPicker({
    source,
    selected,
    onSelect,
    height = 260,
}: {
    source: string;
    selected?: string;
    onSelect(channel: Channel): void;
    height?: number;
}) {
    const [query, setQuery] = useState('');
    return (
        <section className="library-picker" aria-label="Scegli un canale">
            <label>
                Cerca un canale
                <input type="search" value={query} onChange={(e) => setQuery(e.target.value)} />
            </label>
            <ContinuousList<Channel>
                context={`channel-picker:${source}:${query}`}
                label="Canali della lista"
                height={height}
                rowHeight={52}
                blockSize={50}
                load={(offset, signal) =>
                    personalCatalog(
                        { source, content: 'live', kind: 'tv', query, offset, limit: 50 },
                        signal,
                    )
                }
                render={(channel) => (
                    <button
                        className="library-choice"
                        aria-pressed={selected === channel.id}
                        onClick={() => onSelect(channel)}
                    >
                        {channel.name}
                    </button>
                )}
            />
        </section>
    );
}
const today = () => new Date().toLocaleDateString('sv-SE');
export function PersonalGuide({ source }: { source: PersonalSource }) {
    const [selected, select] = useState<Channel | null>(() =>
        readSession(`guide-channel-${source.id}`, null),
    );
    const [picker, setPicker] = useState(false);
    const [day, setDay] = useState(today);
    const [query, setQuery] = useState('');
    const [details, setDetails] = useState<PersonalProgramme | null>(null);
    const detailsHeading = useRef<HTMLHeadingElement>(null);
    const detailsTrigger = useRef<HTMLButtonElement | null>(null);
    useEffect(() => {
        select(readSession(`guide-channel-${source.id}`, null));
        setQuery('');
        const controller = new AbortController();
        void personalCatalog(
            { source: source.id, kind: 'tv', content: 'live', limit: 1 },
            controller.signal,
        )
            .then(({ rows }) => {
                if (controller.signal.aborted) return;
                select((channel) => (channel?.source_id === source.id ? channel : rows[0] || null));
            })
            .catch(() => {});
        return () => controller.abort();
    }, [source.id]);
    useEffect(() => {
        setDetails(null);
    }, [selected?.id, day, query]);
    useEffect(() => {
        if (details) detailsHeading.current?.focus();
    }, [details]);
    const from = new Date(`${day}T00:00:00`).getTime();
    const nextDay = new Date(from);
    nextDay.setDate(nextDay.getDate() + 1);
    return (
        <>
            <div className="library-filters">
                <button
                    className="button secondary"
                    aria-expanded={picker}
                    onClick={() => setPicker(!picker)}
                >
                    {selected?.name || 'Scegli un canale'} · Cambia
                </button>
                <label>
                    Giorno
                    <input
                        type="date"
                        value={day}
                        onChange={(e) => {
                            if (e.target.value) setDay(e.target.value);
                        }}
                    />
                </label>
                <button className="text-button" onClick={() => setDay(today())}>
                    Oggi
                </button>
                <label className="library-guide-search">
                    {source.guide.origin === 'lume' ? 'Cerca nei programmi' : 'Cerca nello storico'}
                    <input
                        type="search"
                        placeholder="Titolo o descrizione"
                        value={query}
                        onChange={(e) => setQuery(e.target.value)}
                    />
                </label>
            </div>
            {picker && (
                <ChannelPicker
                    source={source.id}
                    selected={selected?.id}
                    onSelect={(channel) => {
                        select(channel);
                        writeSession(`guide-channel-${source.id}`, channel);
                        setPicker(false);
                    }}
                />
            )}
            {source.guide.origin === 'lume' && (
                <p className="notice">
                    Guida integrata da Lume. La lista non indica una guida personale; vengono
                    mostrati i programmi dei canali riconosciuti nel feed completo.
                    {source.guide.supplement &&
                        ' EPGShare IT1 completa la programmazione dei canali per cui Lume non ha una guida.'}
                </p>
            )}
            {source.guide.supplement?.error && (
                <p className="notice" role="status">
                    {source.guide.supplement.error}
                </p>
            )}
            {source.guide.loading && (
                <p role="status" className="notice">
                    Aggiornamento della guida in corso. La copia salvata resta disponibile.
                </p>
            )}
            {source.guide.error && (
                <p role="alert" className="library-error">
                    {source.guide.error}{' '}
                    <button
                        className="text-button"
                        onClick={() => void api('/api/library/guide/refresh', { id: source.id })}
                    >
                        Riprova
                    </button>
                </p>
            )}
            {details && (
                <section
                    className="personal-programme-details"
                    aria-labelledby="personal-programme-title"
                >
                    <header>
                        <h2 id="personal-programme-title" tabIndex={-1} ref={detailsHeading}>
                            {details.title}
                        </h2>
                        <button
                            className="text-button"
                            onClick={() => {
                                setDetails(null);
                                if (detailsTrigger.current?.isConnected)
                                    detailsTrigger.current.focus();
                            }}
                        >
                            Chiudi dettagli
                        </button>
                    </header>
                    <p className="programme-meta">
                        {new Date(details.start).toLocaleDateString('it-IT')} · {range(details)}
                    </p>
                    <p>
                        {details.description ||
                            'La guida non contiene una descrizione per questo programma.'}
                    </p>
                </section>
            )}
            {selected && (
                <ContinuousList<PersonalProgramme>
                    context={`guide:${source.id}:${selected.id}:${day}:${query}`}
                    revision={String(source.guide.updated_at)}
                    label={`Programmi di ${selected.name}`}
                    blockSize={50}
                    rowHeight={174}
                    refreshInterval={30_000}
                    load={(offset, signal) =>
                        api<LibraryPage<PersonalProgramme>>(
                            '/api/library/guide/window',
                            { id: selected.id, from, until: nextDay.getTime(), query, offset },
                            { signal },
                        )
                    }
                    render={(programme) => (
                        <article className="personal-programme">
                            <div>
                                <strong>{programme.title}</strong>
                                <time>
                                    {new Date(programme.start).toLocaleDateString('it-IT')} ·{' '}
                                    {range(programme)}
                                </time>
                                <p>{programme.description}</p>
                            </div>
                            <div className="personal-programme-actions">
                                <button
                                    className="text-button"
                                    aria-expanded={details?.event === programme.event}
                                    onClick={(e) => {
                                        detailsTrigger.current = e.currentTarget;
                                        setDetails(programme);
                                    }}
                                >
                                    Dettagli
                                </button>
                                {programme.archive && (
                                    <button
                                        className="button primary"
                                        onClick={() =>
                                            play(selected.id, {
                                                mode: 'archive',
                                                event: programme.event,
                                            })
                                        }
                                    >
                                        Rivedi dall’archivio
                                    </button>
                                )}
                                {programme.ongoing && (
                                    <button
                                        className="button secondary"
                                        onClick={() =>
                                            play(selected.id, {
                                                mode: 'archive',
                                                event: programme.event,
                                            })
                                        }
                                    >
                                        Guarda dall’inizio
                                    </button>
                                )}
                                {!programme.archive && (
                                    <button
                                        className="text-button"
                                        onClick={() => play(selected.id)}
                                    >
                                        Guarda il canale ora
                                    </button>
                                )}
                                {programme.start > Date.now() && (
                                    <button
                                        className="text-button"
                                        onClick={() =>
                                            void savePreferences(
                                                {
                                                    toggle_reminder: {
                                                        channel_id: selected.id,
                                                        title: programme.title,
                                                        start: programme.start,
                                                    },
                                                },
                                                'Promemoria aggiornato.',
                                            )
                                        }
                                    >
                                        Promemoria
                                    </button>
                                )}
                            </div>
                        </article>
                    )}
                    empty={
                        <p className="notice">
                            {source.guide.loading
                                ? 'Attendi il completamento della guida.'
                                : source.guide.origin === 'lume'
                                  ? source.guide.supplement
                                      ? 'Le guide Lume ed EPGShare IT1 non contengono programmi per questo canale, data o ricerca.'
                                      : 'Il feed Lume non contiene programmi per questo canale, data o ricerca.'
                                  : source.guide.matched
                                    ? 'Nessun programma per questa data o ricerca.'
                                    : 'Non ci sono associazioni per questo canale. Configura la guida e le associazioni in Le mie liste.'}
                        </p>
                    }
                />
            )}
        </>
    );
}
