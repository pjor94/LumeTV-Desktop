import { watchOnTv } from '../companion';
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { api } from '../api';
import { app, play, navigate, playback, setQuery } from '../app';
import { library, videoSelection, errorMessage, type LibraryPage } from '../personal';
import { useStore, useStoreValue } from '../store';
import { ContinuousList } from '../components/ContinuousList';
import { EmptyState, Heading, Icon } from '../components/common';
import { GroupPicker, SourceSelector } from './LibraryCatalog';
import {
    changedVideos,
    videoAction,
    videoArtwork,
    artworkDataUrl,
    videoChanges,
    videoInfo,
    videoTime,
    type Candidate,
    type MetadataResult,
    type MetadataSettings,
    type VideoDetail,
    type VideoItem,
    type VideoQuery,
    type VideoShelf,
} from '../video';

function useResource<T>(
    path: string,
    body: unknown,
    enabled: boolean,
    revision: string | number = '',
) {
    const requestKey = `${path}:${JSON.stringify(body)}`;
    const key = `${requestKey}:${revision}`;
    const [value, setValue] = useState<{
        key: string;
        requestKey: string;
        data?: T;
        error?: string;
    }>();
    useEffect(() => {
        if (!enabled) return;
        const controller = new AbortController();
        void api<T>(path, body, { signal: controller.signal })
            .then((data) => {
                if (!controller.signal.aborted) setValue({ key, requestKey, data });
            })
            .catch((error) => {
                if (!controller.signal.aborted)
                    setValue((old) => ({
                        key,
                        requestKey,
                        data: old?.requestKey === requestKey ? old.data : undefined,
                        error: errorMessage(error),
                    }));
            });
        return () => controller.abort();
        // The serialized key captures the request's full input.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [key, enabled]);
    return value?.requestKey === requestKey
        ? { ...value, loading: enabled && value.key !== key }
        : { loading: enabled, data: undefined, error: undefined };
}

function Artwork({
    item,
    backdrop = false,
    revision = 0,
}: {
    item: VideoItem;
    backdrop?: boolean;
    revision?: number;
}) {
    const available = backdrop ? item.backdrop : item.poster;
    const [picture, setPicture] = useState({ id: '', url: '' });
    const element = useRef<HTMLSpanElement>(null);
    const [visible, setVisible] = useState(false);
    useEffect(() => {
        const observer = new IntersectionObserver(
            (entries) => {
                if (entries.some((e) => e.isIntersecting)) {
                    setVisible(true);
                    observer.disconnect();
                }
            },
            { rootMargin: '200px' },
        );
        if (element.current) observer.observe(element.current);
        return () => observer.disconnect();
    }, [item.id]);
    useEffect(() => {
        if (!available || !visible) return;
        const controller = new AbortController();
        const path = `/art/video/${encodeURIComponent(item.id)}/${backdrop ? 'backdrop' : 'poster'}`;
        void videoArtwork(`${path}:${item.online}:${revision}`, path, controller.signal)
            .then((blob) => artworkDataUrl(blob, controller.signal))
            .then((url) => {
                if (controller.signal.aborted) return;
                setPicture({ id: item.id, url });
            })
            .catch(() => {});
        return () => controller.abort();
    }, [item.id, available, backdrop, revision, visible, item.online]);
    return (
        <span className="video-art-frame" ref={element}>
            {picture.id === item.id && available && picture.url ? (
                <img
                    className="video-art"
                    src={picture.url}
                    alt=""
                    onError={() => setPicture({ id: item.id, url: '' })}
                />
            ) : (
                <span className="video-art-fallback" aria-hidden="true">
                    <Icon name={item.series ? 'guide' : 'play'} />
                    <span>{backdrop ? '' : item.series_title || item.title}</span>
                </span>
            )}
        </span>
    );
}
function Poster({
    item,
    open,
    episode = false,
    preview,
}: {
    item: VideoItem;
    preview?(item: VideoItem): void;
    open(item: VideoItem, episode?: boolean): void;
    episode?: boolean;
}) {
    const series = !!item.series && !episode;
    return (
        <button
            className="video-card"
            onFocus={() => preview?.(item)}
            onPointerEnter={() => preview?.(item)}
            onClick={() => open(item, episode)}
            aria-label={`Dettagli di ${series ? item.series_title : item.title}`}
        >
            <span className="video-poster">
                <Artwork item={item} revision={videoChanges.get().revision} />
                {(item.completed || item.saved || series) && (
                    <span className="video-badge">
                        {item.completed && !series
                            ? 'Visto'
                            : series
                              ? `${item.episodes} episodi`
                              : 'Nella mia lista'}
                    </span>
                )}
                {!item.completed && item.position >= 5 && (
                    <span
                        className="video-progress"
                        style={
                            {
                                '--progress': `${Math.min(100, (item.position / (item.watched_duration || item.duration || item.position * 2)) * 100)}%`,
                            } as React.CSSProperties
                        }
                    />
                )}
            </span>
            <strong>{series ? item.series_title : item.title}</strong>
            <small>
                {episode && item.position >= 5 && !item.completed
                    ? `Riprendi da ${videoTime(item.position)}`
                    : videoInfo(item, series)}
            </small>
        </button>
    );
}

function OnlineSettings({ close }: { close(): void }) {
    const [settings, setSettings] = useState<MetadataSettings>();
    const [token, setToken] = useState('');
    const [busy, setBusy] = useState(false);
    const [message, setMessage] = useState('');
    useEffect(() => {
        const controller = new AbortController();
        void api<MetadataSettings>('/api/library/video/settings', {}, { signal: controller.signal })
            .then(setSettings)
            .catch((e) => {
                if (!controller.signal.aborted) setMessage(errorMessage(e));
            });
        return () => controller.abort();
    }, []);
    const update = async (patch: object) => {
        const previous = settings;
        if ('enabled' in patch && typeof patch.enabled === 'boolean' && settings)
            setSettings({ ...settings, enabled: patch.enabled });
        setBusy(true);
        setMessage('');
        try {
            setSettings(await api('/api/library/video/settings', patch));
            setToken('');
            changedVideos();
            setMessage('Preferenze salvate.');
        } catch (e) {
            setSettings(previous);
            setMessage(errorMessage(e));
        } finally {
            setBusy(false);
        }
    };
    return (
        <section className="video-settings" aria-label="Informazioni online">
            <header>
                <h2>Informazioni online</h2>
                <button className="text-button" onClick={close}>
                    Chiudi
                </button>
            </header>
            <p>
                Recupera trama, cast e immagini da TMDB quando apri un titolo. La ricerca invia il
                titolo e, quando disponibile, l’anno. I dati già scaricati restano disponibili
                offline.
            </p>
            <label className="video-toggle">
                <input
                    type="checkbox"
                    checked={settings?.enabled || false}
                    disabled={busy || !settings?.configured}
                    onChange={(e) => void update({ enabled: e.target.checked })}
                />
                Usa le informazioni di TMDB
            </label>
            <p>
                {settings?.configured
                    ? 'TMDB è configurato.'
                    : 'Per attivare le informazioni online, configura il Read Access Token del tuo account TMDB.'}
            </p>
            <form
                onSubmit={(e) => {
                    e.preventDefault();
                    void update({ token });
                }}
            >
                <label>
                    Read Access Token TMDB
                    <input
                        type="password"
                        autoComplete="off"
                        spellCheck={false}
                        value={token}
                        onChange={(e) => setToken(e.target.value)}
                        placeholder={
                            settings?.configured
                                ? 'Sostituisci il token configurato'
                                : 'Incolla il token'
                        }
                    />
                </label>
                <button className="button secondary" disabled={busy || !token.trim()}>
                    Salva token
                </button>
            </form>
            <button
                className="text-button"
                disabled={busy}
                onClick={() => void update({ clear: true })}
            >
                Svuota la cache online
            </button>
            <small>Le associazioni scelte manualmente vengono conservate.</small>
            <p role="status">{busy ? 'Salvataggio…' : message}</p>
            <a
                className="video-attribution"
                href="https://www.themoviedb.org"
                target="_blank"
                rel="noreferrer"
            >
                <img src="/assets/tmdb.svg" alt="TMDB" width="100" height="18" />
            </a>
            <small>This product uses the TMDB API but is not endorsed or certified by TMDB.</small>
        </section>
    );
}

function MetadataEditor({ item, refresh }: { item: VideoItem; refresh(): void }) {
    const [settings, setSettings] = useState<MetadataSettings>();
    const [result, setResult] = useState<MetadataResult>();
    const [editing, setEditing] = useState(false);
    const [query, setSearch] = useState('');
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState('');
    const revision = useStore(videoChanges).revision;
    const generation = useRef(0);
    const actionRequest = useRef<AbortController | null>(null);
    useEffect(() => {
        generation.current++;
        actionRequest.current?.abort();
        const controller = new AbortController();
        setBusy(false);
        setSearch('');
        setResult(undefined);
        setSettings(undefined);
        setError('');
        setEditing(false);
        void api<MetadataSettings>('/api/library/video/settings', {}, { signal: controller.signal })
            .then(async (value) => {
                if (controller.signal.aborted) return;
                setSettings(value);
                if (value.enabled && value.configured) {
                    setBusy(true);
                    const data = await api<MetadataResult>(
                        '/api/library/video/metadata',
                        { id: item.id },
                        { signal: controller.signal },
                    );
                    if (!controller.signal.aborted) {
                        setResult(data);
                        refresh();
                    }
                }
            })
            .catch((e) => {
                if (!controller.signal.aborted) setError(errorMessage(e));
            })
            .finally(() => {
                if (!controller.signal.aborted) setBusy(false);
            });
        return () => {
            controller.abort();
            actionRequest.current?.abort();
            generation.current++;
        };
    }, [item.id, revision]);
    const choose = async (candidate: Candidate | null) => {
        const epoch = generation.current;
        actionRequest.current?.abort();
        const controller = new AbortController();
        actionRequest.current = controller;
        setBusy(true);
        setError('');
        try {
            const data = await api<MetadataResult>(
                '/api/library/video/choose',
                {
                    id: item.id,
                    candidate: candidate?.id ?? null,
                },
                { signal: controller.signal },
            );
            if (controller.signal.aborted || epoch !== generation.current) return;
            setResult(data);
            setEditing(false);
            refresh();
        } catch (e) {
            if (!controller.signal.aborted && epoch === generation.current)
                setError(errorMessage(e));
        } finally {
            if (!controller.signal.aborted && epoch === generation.current) setBusy(false);
        }
    };
    return (
        <section className="video-online" aria-label="Associazione TMDB">
            <p role="status">
                {busy
                    ? 'Recupero informazioni da TMDB…'
                    : error ||
                      (result?.status === 'ambiguous'
                          ? 'Ci sono più titoli possibili. Scegli quello corretto.'
                          : result?.status === 'not_found'
                            ? 'Nessun titolo trovato. Puoi cercarlo manualmente.'
                            : result?.status === 'excluded'
                              ? 'Per questo titolo usi le informazioni della lista.'
                              : settings?.enabled && settings.configured
                                ? item.online
                                    ? 'Informazioni da TMDB.'
                                    : ''
                                : 'Le informazioni online si attivano dalle opzioni Video.')}
            </p>
            {settings?.enabled && settings.configured && (
                <button
                    className="text-button"
                    disabled={busy}
                    aria-expanded={editing}
                    onClick={() => setEditing(!editing)}
                >
                    {editing ? 'Chiudi ricerca' : 'Scegli un altro titolo su TMDB'}
                </button>
            )}
            {(editing || result?.status === 'ambiguous') && (
                <>
                    <form
                        onSubmit={(e) => {
                            e.preventDefault();
                            setBusy(true);
                            setError('');
                            const epoch = generation.current;
                            actionRequest.current?.abort();
                            const controller = new AbortController();
                            actionRequest.current = controller;
                            void api<MetadataResult>(
                                '/api/library/video/metadata',
                                {
                                    id: item.id,
                                    query,
                                },
                                { signal: controller.signal },
                            )
                                .then((data) => {
                                    if (!controller.signal.aborted && epoch === generation.current)
                                        setResult(data);
                                })
                                .catch((e) => {
                                    if (!controller.signal.aborted && epoch === generation.current)
                                        setError(errorMessage(e));
                                })
                                .finally(() => {
                                    if (!controller.signal.aborted && epoch === generation.current)
                                        setBusy(false);
                                });
                        }}
                    >
                        <label>
                            Cerca su TMDB
                            <input
                                type="search"
                                value={query}
                                onChange={(e) => setSearch(e.target.value)}
                                placeholder={item.series_title || item.title}
                                maxLength={300}
                            />
                        </label>
                        <button className="button secondary" disabled={busy}>
                            Cerca
                        </button>
                    </form>
                    <div className="video-candidates">
                        {result?.candidates.map((candidate) => (
                            <button
                                key={`${candidate.kind}:${candidate.id}`}
                                className="library-choice"
                                disabled={busy}
                                onClick={() => void choose(candidate)}
                            >
                                {candidate.title}
                                {candidate.year ? ` · ${candidate.year}` : ''}
                            </button>
                        ))}
                    </div>
                    <button
                        className="text-button"
                        disabled={busy}
                        onClick={() => void choose(null)}
                    >
                        Usa solo le informazioni della lista
                    </button>
                </>
            )}
        </section>
    );
}

function Details({
    id,
    episode,
    back,
    open,
    visible,
}: {
    id: string;
    episode: boolean;
    back(): void;
    open(item: VideoItem, episode?: boolean): void;
    visible: boolean;
}) {
    const [local, setLocal] = useState(0);
    const [error, setError] = useState('');
    const [busy, setBusy] = useState(false);
    const change = useStore(videoChanges).revision;
    const sources = useStore(library);
    const request = useResource<VideoDetail>(
        '/api/library/video/detail',
        { id },
        visible,
        `${local}:${change}:${sources.revision}:${app.get().preferences.favorites.join('|')}`,
    );
    const item = request.data?.item;
    const series = !!item?.series && !episode;
    const target = series ? request.data?.target_item || item : item;
    const [season, setSeason] = useState<number | null>(null);
    const [seasonPage, setSeasonPage] = useState(0);
    const seasonMemory = useRef(new Map<string, number | null>());
    const selectSeason = (value: number | null) => {
        if (item?.series) seasonMemory.current.set(`${item.source}:${item.series}`, value);
        setSeason(value);
    };
    const heading = useRef<HTMLHeadingElement>(null);
    useEffect(() => {
        setSeason(null);
        setSeasonPage(0);
        setError('');
    }, [id, episode]);
    useEffect(() => {
        if (item && series) {
            const memoryKey = `${item.source}:${item.series}`;
            const selected = seasonMemory.current.has(memoryKey)
                ? seasonMemory.current.get(memoryKey)!
                : (request.data?.target_item?.season ?? request.data?.seasons[0]?.season ?? null);
            setSeason(selected);
            setSeasonPage(
                Math.max(
                    0,
                    Math.floor(
                        (request.data?.seasons.findIndex((s) => s.season === selected) || 0) / 8,
                    ),
                ),
            );
        }
        heading.current?.focus({ preventScroll: true });
    }, [id, !!item, series]);
    const action = async (name: 'saved' | 'completed' | 'hidden', enabled: boolean) => {
        if (!item) return;
        setBusy(true);
        setError('');
        try {
            await videoAction(item, name, enabled);
        } catch (e) {
            setError(errorMessage(e));
        } finally {
            setBusy(false);
        }
    };
    const seasons = request.data?.seasons || [];
    const seasonWindow = seasons.slice(seasonPage * 8, seasonPage * 8 + 8);
    if (season !== null && !seasonWindow.some((s) => s.season === season)) {
        const selected = seasons.find((s) => s.season === season);
        if (selected) {
            if (seasonWindow.length >= 8) seasonWindow.pop();
            seasonWindow.unshift(selected);
        }
    }
    return (
        <article className="video-detail">
            <button className="text-button video-back" onClick={back}>
                <Icon name="arrow" />
                Torna ai video
            </button>
            {request.error && (
                <p role="alert" className="library-error">
                    {request.error}
                    <button className="text-button" onClick={() => setLocal(local + 1)}>
                        Riprova
                    </button>
                </p>
            )}
            {!item ? (
                !request.error && (
                    <div className="video-detail-loading" role="status">
                        Caricamento del titolo…
                    </div>
                )
            ) : (
                <>
                    <div className="video-detail-hero">
                        {item.backdrop && (
                            <div className="video-backdrop">
                                <Artwork item={item} backdrop revision={local + change} />
                            </div>
                        )}
                        <div className="video-detail-poster">
                            <Artwork item={item} revision={local + change} />
                        </div>
                        <div className="video-detail-copy">
                            <h1 tabIndex={-1} ref={heading}>
                                {series ? item.series_title : item.title}
                            </h1>
                            <p className="video-facts">{videoInfo(item, series)}</p>
                            {!!item.genres.length && (
                                <p className="video-genres">{item.genres.join(' · ')}</p>
                            )}
                            <p className="video-description">
                                {item.description ||
                                    (series
                                        ? 'Scegli un episodio dalla tua lista.'
                                        : 'La lista non include una trama per questo titolo.')}
                            </p>
                            {item.overview_language === 'en' && (
                                <small>Trama disponibile in inglese</small>
                            )}
                            {!!item.directors?.length && (
                                <p className="video-cast">
                                    <strong>
                                        {item.metadata_type === 'tv' ||
                                        item.series ||
                                        item.kind === 'episode'
                                            ? 'Creazione'
                                            : 'Regia'}
                                    </strong>{' '}
                                    {item.directors.join(', ')}
                                </p>
                            )}
                            {item.cast.length > 0 && (
                                <p className="video-cast">
                                    <strong>Cast</strong> {item.cast.join(', ')}
                                </p>
                            )}
                            <div className="video-actions">
                                <button
                                    className="button secondary"
                                    onClick={() =>
                                        watchOnTv(
                                            series ? request.data!.target : item.id,
                                            target?.completed ? 0 : target?.position || 0,
                                        )
                                    }
                                >
                                    <Icon name="tv" />
                                    Guarda sulla TV
                                </button>
                                <button
                                    className="button primary"
                                    onClick={() =>
                                        play(series ? request.data!.target : item.id, {
                                            mode: 'vod',
                                        })
                                    }
                                >
                                    <Icon name="play" />
                                    {target?.completed
                                        ? 'Riguarda'
                                        : target && target.position >= 5
                                          ? `Riprendi da ${videoTime(target.position)}`
                                          : 'Guarda'}
                                </button>
                                <button
                                    className="button secondary"
                                    disabled={busy}
                                    aria-pressed={item.saved}
                                    onClick={() => void action('saved', !item.saved)}
                                >
                                    <Icon name="star" />
                                    {item.saved ? 'Nella mia lista' : 'La mia lista'}
                                </button>
                                {target && (target.position >= 5 || target.completed) && (
                                    <button
                                        className="text-button"
                                        onClick={() =>
                                            play(series ? request.data!.target : item.id, {
                                                mode: 'vod',
                                                position: 0,
                                            })
                                        }
                                    >
                                        Dall’inizio
                                    </button>
                                )}
                            </div>
                            {series && target && (
                                <p className="video-next-label">
                                    {target.completed ? 'Riguarda' : 'Da vedere'} · {target.title}
                                </p>
                            )}
                            {!series && (
                                <div className="video-state-actions">
                                    <button
                                        className="text-button"
                                        disabled={busy}
                                        onClick={() => void action('completed', !item.completed)}
                                    >
                                        {item.completed
                                            ? 'Segna come non visto'
                                            : 'Segna come visto'}
                                    </button>
                                    {item.position >= 5 && !item.completed && (
                                        <button
                                            className="text-button"
                                            disabled={busy}
                                            onClick={() => void action('hidden', !item.hidden)}
                                        >
                                            {item.hidden
                                                ? 'Mostra in Continua a guardare'
                                                : 'Rimuovi da Continua a guardare'}
                                        </button>
                                    )}
                                    {request.data?.next && (
                                        <button
                                            className="text-button"
                                            onClick={() =>
                                                play(request.data!.next!, { mode: 'vod' })
                                            }
                                        >
                                            Prossimo episodio
                                            <Icon name="arrow" />
                                        </button>
                                    )}
                                </div>
                            )}
                            {error && (
                                <p className="library-error" role="alert">
                                    {error}
                                </p>
                            )}
                        </div>
                    </div>
                    {series && (
                        <section className="video-episodes" aria-label="Episodi della serie">
                            <header className="section-heading">
                                <h2>Episodi</h2>
                            </header>
                            <div className="video-season-bar" aria-label="Stagioni">
                                <button
                                    className="category-button"
                                    aria-pressed={season === null}
                                    onClick={() => selectSeason(null)}
                                >
                                    Tutte le stagioni
                                </button>
                                {seasonWindow.map((s) => (
                                    <button
                                        key={s.season}
                                        className="category-button"
                                        aria-pressed={season === s.season}
                                        onClick={() => selectSeason(s.season)}
                                    >
                                        {s.season ? `Stagione ${s.season}` : 'Altri episodi'}{' '}
                                        <small>{s.episodes}</small>
                                    </button>
                                ))}
                                {seasonPage > 0 && (
                                    <button
                                        className="text-button"
                                        onClick={() => setSeasonPage(seasonPage - 1)}
                                    >
                                        Stagioni precedenti
                                    </button>
                                )}
                                {(seasonPage + 1) * 8 < seasons.length && (
                                    <button
                                        className="text-button"
                                        onClick={() => setSeasonPage(seasonPage + 1)}
                                    >
                                        Altre stagioni
                                    </button>
                                )}
                            </div>
                            <ContinuousList<VideoItem>
                                context={`episodes:${item.source}:${item.series}:${season}`}
                                revision={`${sources.revision}:${change}:${local}`}
                                label="Episodi"
                                rowHeight={84}
                                height={460}
                                blockSize={50}
                                active={visible}
                                load={(offset, signal) =>
                                    api<LibraryPage<VideoItem>>(
                                        '/api/library/video/page',
                                        {
                                            source: item.source,
                                            series: item.series,
                                            season,
                                            episodes: true,
                                            offset,
                                            limit: 50,
                                        },
                                        { signal },
                                    )
                                }
                                render={(entry) => (
                                    <div className="video-episode">
                                        <button
                                            className="video-episode-play"
                                            onClick={() => play(entry.id, { mode: 'vod' })}
                                        >
                                            <span className="video-episode-number">
                                                {entry.episode ?? '–'}
                                            </span>
                                            <span>
                                                <strong>{entry.title}</strong>
                                                <small>
                                                    {entry.completed
                                                        ? 'Visto'
                                                        : entry.position >= 5
                                                          ? `Riprendi da ${videoTime(entry.position)}`
                                                          : videoInfo(entry)}
                                                </small>
                                            </span>
                                            <Icon name="play" />
                                        </button>
                                        <button
                                            className="icon-button"
                                            aria-label={`Dettagli di ${entry.title}`}
                                            onClick={() => open(entry, true)}
                                        >
                                            <Icon name="info" />
                                        </button>
                                    </div>
                                )}
                            />
                        </section>
                    )}
                    <MetadataEditor item={item} refresh={() => setLocal((v) => v + 1)} />
                </>
            )}
        </article>
    );
}

function HomePreview({ item, open }: { item: VideoItem; open(item: VideoItem): void }) {
    const series = !!item.series;
    return (
        <section className="video-home-preview" aria-label="Titolo in evidenza">
            <div className="video-home-preview-art">
                {(item.poster || item.backdrop) && (
                    <Artwork
                        item={item}
                        backdrop={item.backdrop}
                        revision={videoChanges.get().revision}
                    />
                )}
            </div>
            <div className="video-home-preview-copy">
                <h2>{series ? item.series_title : item.title}</h2>
                <p className="video-facts">{videoInfo(item, series)}</p>
                <p className="video-preview-description">
                    {item.description ||
                        (series
                            ? 'Tutti gli episodi disponibili nella tua lista.'
                            : 'Un film dalla tua libreria personale.')}
                </p>
                <button className="button primary" onClick={() => open(item)}>
                    Apri scheda <Icon name="arrow" />
                </button>
            </div>
        </section>
    );
}
function CategoryShelf({
    group,
    source,
    version,
    active,
    open,
    preview,
    browse,
}: {
    group: string;
    source: string;
    version: string;
    active: boolean;
    open(item: VideoItem): void;
    preview(item: VideoItem): void;
    browse(group: string): void;
}) {
    const root = useRef<HTMLElement>(null);
    const [visible, setVisible] = useState(false);
    useEffect(() => {
        if (!active || !root.current) return;
        const observer = new IntersectionObserver(
            ([entry]) => {
                if (entry.isIntersecting) {
                    setVisible(true);
                    observer.disconnect();
                }
            },
            { rootMargin: '250px' },
        );
        observer.observe(root.current);
        return () => observer.disconnect();
    }, [active]);
    const page = useResource<LibraryPage<VideoItem>>(
        '/api/library/video/page',
        { source, group, limit: 12 },
        active && visible,
        version,
    );
    return (
        <section
            ref={root}
            className="video-shelf video-category-shelf"
            aria-label={`Categoria ${group}`}
        >
            <header className="section-heading">
                <h2>{group}</h2>
                <button className="text-button" onClick={() => browse(group)}>
                    Mostra tutti <Icon name="arrow" />
                </button>
            </header>
            <div className="video-shelf-posters">
                {page.data?.rows.map((item) => (
                    <Poster key={item.id} item={item} open={open} preview={preview} />
                ))}
            </div>
            {page.loading && <p role="status">Carico i titoli…</p>}
            {page.error && <p role="alert">{page.error}</p>}
        </section>
    );
}
function CategoryShelves({
    source,
    version,
    active,
    open,
    preview,
    browse,
}: {
    source: string;
    version: string;
    active: boolean;
    open(item: VideoItem): void;
    preview(item: VideoItem): void;
    browse(group: string): void;
}) {
    const [offset, setOffset] = useState(0);
    const [limit, setLimit] = useState(8);
    const groups = useResource<LibraryPage<string>>(
        '/api/library/groups',
        { source, content: 'vod', kind: 'tv', offset },
        active,
        version,
    );
    return (
        <>
            {groups.data?.rows.slice(0, limit).map((group) => (
                <CategoryShelf
                    key={group}
                    group={group}
                    source={source}
                    active={active}
                    version={version}
                    open={open}
                    preview={preview}
                    browse={browse}
                />
            ))}
            {groups.data &&
                offset + Math.min(limit, groups.data.rows.length) < groups.data.total && (
                    <button
                        className="button secondary"
                        onClick={() => {
                            if (limit < groups.data!.rows.length) setLimit((v) => v + 8);
                            else {
                                setOffset((v) => v + 50);
                                setLimit(8);
                            }
                        }}
                    >
                        Mostra altre categorie
                    </button>
                )}
            {offset > 0 && (
                <button
                    className="text-button"
                    onClick={() => {
                        setOffset((v) => Math.max(0, v - 50));
                        setLimit(8);
                    }}
                >
                    Categorie precedenti
                </button>
            )}
        </>
    );
}

export function Videos() {
    const sources = useStore(library);
    const selection = useStore(videoSelection);
    const revision = useStore(videoChanges).revision;
    const page = useStoreValue(app, (state) => state.page);
    const active = page !== 'watch';
    const [filter, setFilter] = useState('all');
    const [preview, setPreview] = useState<{ version: string; item: VideoItem }>();
    const [order, setOrder] = useState('playlist');
    const [groups, showGroups] = useState(false);
    const [options, showOptions] = useState(false);
    const [detail, setDetail] = useState<{ id: string; episode: boolean }>();
    const detailHistory = useRef<
        { detail: { id: string; episode: boolean } | undefined; scroll: number }[]
    >([]);
    const [retry, setRetry] = useState(0);
    const priorScroll = useRef(0);
    const previousSource = useRef(selection.source);
    const showHome =
        filter === 'all' && !selection.query && !selection.group && order === 'playlist';
    const version = `${sources.revision}:${revision}:${app.get().preferences.favorites.join('|')}:${retry}`;
    const home = useResource<{ shelves: VideoShelf[] }>(
        '/api/library/video/home',
        { source: selection.source },
        active && !detail && showHome,
        version,
    );
    useEffect(() => {
        if (previousSource.current !== selection.source) {
            setDetail(undefined);
            detailHistory.current = [];
            setFilter('all');
            previousSource.current = selection.source;
        }
        if (
            selection.source &&
            sources.ready &&
            !sources.sources.some(
                (s) =>
                    s.id === selection.source &&
                    s.enabled &&
                    (s.has_vod || s.report.count > s.report.radio),
            )
        )
            videoSelection.set({ source: '', group: '' });
    }, [selection.source, sources.revision, sources.ready, sources.sources]);
    useEffect(() => {
        if (selection.query) {
            setDetail(undefined);
            detailHistory.current = [];
        }
    }, [selection.query]);
    useEffect(() => {
        if (active) changedVideos();
    }, [active]);
    useEffect(() => {
        if (!active) return;
        const timer = setInterval(changedVideos, 30_000);
        return () => clearInterval(timer);
    }, [active]);
    const open = (item: VideoItem, episode = false) => {
        priorScroll.current = document.querySelector('.content-scroll')?.scrollTop || 0;
        detailHistory.current.push({ detail, scroll: priorScroll.current });
        setDetail({ id: item.id, episode });
    };
    useLayoutEffect(() => {
        if (detail)
            document.querySelector('.content-scroll')?.scrollTo({ top: 0, behavior: 'instant' });
    }, [detail]);
    const back = () => {
        const previous = detailHistory.current.pop();
        setDetail(previous?.detail);
        requestAnimationFrame(() => {
            const root = document.querySelector('.content-scroll');
            if (root) root.scrollTop = previous?.scroll || 0;
        });
    };
    const query: VideoQuery = {
        source: selection.source,
        group: selection.group,
        query: selection.query,
        order,
        kind: filter === 'movies' ? 'movie' : filter === 'series' ? 'episode' : '',
        shelf: ['continue', 'saved', 'added'].includes(filter) ? filter : '',
    };
    if (detail)
        return (
            <Details
                id={detail.id}
                episode={detail.episode}
                back={back}
                open={open}
                visible={active}
            />
        );
    return (
        <>
            <Heading
                title="I tuoi film. Le tue serie."
                subtitle="Una libreria dalle tue liste, da riprendere quando vuoi."
            />
            <SourceSelector videos />
            <div className="video-navigation" aria-label="Raccolte video">
                {[
                    ['all', 'Tutti'],
                    ['movies', 'Film'],
                    ['series', 'Serie'],
                    ['continue', 'Continua a guardare'],
                    ['saved', 'La mia lista'],
                    ['added', 'Aggiunti di recente'],
                ].map(([value, label]) => (
                    <button
                        key={value}
                        className="category-button"
                        aria-pressed={filter === value}
                        onClick={() => setFilter(value)}
                    >
                        {label}
                    </button>
                ))}
            </div>
            <div className="library-filters video-filters">
                <button
                    className="button secondary"
                    aria-expanded={groups}
                    onClick={() => showGroups(!groups)}
                >
                    {selection.group || 'Tutti i gruppi'}
                    <Icon name="chevron" />
                </button>
                <label>
                    Ordine{' '}
                    <select
                        className="category-select"
                        aria-label="Ordine dei video"
                        value={order}
                        onChange={(e) => setOrder(e.target.value)}
                    >
                        <option value="playlist">Della lista</option>
                        <option value="title">Titolo A–Z</option>
                        <option value="added">Aggiunti di recente</option>
                    </select>
                </label>
                <button
                    className="text-button"
                    aria-expanded={options}
                    onClick={() => showOptions(!options)}
                >
                    <Icon name="settings" />
                    Informazioni online
                </button>
                {(selection.group || selection.query) && (
                    <button
                        className="text-button"
                        onClick={() => {
                            videoSelection.set({ group: '', query: '' });
                            setQuery('');
                        }}
                    >
                        Azzera filtri
                    </button>
                )}
            </div>
            {options && <OnlineSettings close={() => showOptions(false)} />}
            {groups && (
                <GroupPicker
                    source={selection.source}
                    content="vod"
                    kind="tv"
                    selected={selection.group}
                    onSelect={(group) => {
                        videoSelection.set({ group });
                        showGroups(false);
                    }}
                    onClose={() => showGroups(false)}
                />
            )}
            {showHome ? (
                <div className="video-home">
                    {home.loading && !home.data && (
                        <div className="video-home-loading" role="status">
                            Caricamento della libreria…
                        </div>
                    )}
                    {home.error && (
                        <p className="library-error" role="alert">
                            {home.error}{' '}
                            <button className="text-button" onClick={() => setRetry(retry + 1)}>
                                Riprova
                            </button>
                        </p>
                    )}
                    {home.data?.shelves.length ? (
                        <HomePreview
                            item={
                                preview?.version === version
                                    ? preview.item
                                    : home.data.shelves[0].rows[0]
                            }
                            open={open}
                        />
                    ) : null}
                    {home.data?.shelves.map((shelf) => (
                        <section className="video-shelf" key={shelf.id} aria-label={shelf.title}>
                            <header className="section-heading">
                                <h2>
                                    {shelf.title} <span>{shelf.total.toLocaleString('it-IT')}</span>
                                </h2>
                                <button
                                    className="text-button"
                                    onClick={() =>
                                        setFilter(shelf.id === 'all' ? 'browse' : shelf.id)
                                    }
                                >
                                    Mostra tutti
                                    <Icon name="arrow" />
                                </button>
                            </header>
                            <div className="video-shelf-posters">
                                {shelf.rows.map((item) => (
                                    <Poster
                                        key={item.id}
                                        item={item}
                                        open={open}
                                        preview={(item) => setPreview({ version, item })}
                                        episode={shelf.id === 'continue'}
                                    />
                                ))}
                            </div>
                        </section>
                    ))}
                    {!!home.data?.shelves.length && (
                        <CategoryShelves
                            key={`${selection.source}:${version}`}
                            source={selection.source}
                            version={version}
                            active={active}
                            open={open}
                            preview={(item) => setPreview({ version, item })}
                            browse={(group) => {
                                videoSelection.set({ group });
                                setFilter('browse');
                            }}
                        />
                    )}
                    {!home.loading && home.data && !home.data.shelves.length && (
                        <EmptyState
                            symbol="play"
                            title="La tua libreria è pronta per i video."
                            description="Importa una lista con film e serie, oppure controlla il tipo di contenuto nelle impostazioni della lista."
                        >
                            <button
                                className="button primary"
                                onClick={() => navigate('playlists')}
                            >
                                Gestisci liste
                            </button>
                        </EmptyState>
                    )}
                </div>
            ) : (
                <>
                    <div className="section-heading">
                        <h2>
                            {selection.query
                                ? `Risultati per “${selection.query}”`
                                : selection.group ||
                                  (
                                      {
                                          movies: 'Film',
                                          series: 'Serie',
                                          continue: 'Continua a guardare',
                                          saved: 'La mia lista',
                                          added: 'Aggiunti di recente',
                                      } as Record<string, string>
                                  )[filter] ||
                                  'Tutti i video'}
                        </h2>
                        <button
                            className="text-button"
                            onClick={() => {
                                setFilter('all');
                                setOrder('playlist');
                                videoSelection.set({ group: '', query: '' });
                                setQuery('');
                            }}
                        >
                            Torna alla libreria
                        </button>
                    </div>
                    <ContinuousList<VideoItem>
                        context={`videos:${JSON.stringify(query)}`}
                        revision={version}
                        label="Video delle mie liste"
                        itemNames={['titolo', 'titoli']}
                        layout="videos"
                        active={active}
                        load={(offset, signal) =>
                            api<LibraryPage<VideoItem>>(
                                '/api/library/video/page',
                                { ...query, offset, limit: 100 },
                                { signal },
                            )
                        }
                        blockSize={100}
                        render={(item) => (
                            <Poster item={item} open={open} episode={filter === 'continue'} />
                        )}
                        empty={
                            <EmptyState
                                symbol="play"
                                title="Nessun titolo in questa raccolta."
                                description={
                                    selection.query || selection.group
                                        ? 'Prova un altro nome o azzera i filtri.'
                                        : filter === 'continue'
                                          ? 'I video che inizi a guardare compariranno qui.'
                                          : filter === 'saved'
                                            ? 'Apri un titolo e aggiungilo alla tua lista.'
                                            : 'Controlla le liste abilitate o scegli un’altra raccolta.'
                                }
                            />
                        }
                    />
                </>
            )}
        </>
    );
}

export function VideoNextEpisode() {
    const state = useStore(playback.store);
    const visible = state.mode === 'vod' && !!state.current;
    const detail = useResource<VideoDetail>(
        '/api/library/video/detail',
        { id: state.current },
        visible,
        state.ended ? 'ended' : 'playing',
    );
    if (!visible || !detail.data?.next) return null;
    return (
        <button
            className="button secondary"
            onClick={() => play(detail.data!.next!, { mode: 'vod' })}
        >
            <Icon name="play" />
            Prossimo episodio
        </button>
    );
}
