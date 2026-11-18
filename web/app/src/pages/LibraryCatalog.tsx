import { useEffect, useState } from 'react';
import { app, navigate, openGuide, play, playback, setQuery } from '../app';
import { api } from '../api';
import {
    library,
    librarySelection,
    videoSelection,
    personalRadioSelection,
    personalCatalog,
    writeSession,
    type LibraryPage,
    type PersonalHome,
} from '../personal';
import { useStore, useStoreValue } from '../store';
import { ContinuousList } from '../components/ContinuousList';
import { CatalogHero, RecentChannels } from '../components/CatalogHome';
import {
    ChannelCard,
    EmptyState,
    FavoriteButton,
    Heading,
    Icon,
    StationLogo,
} from '../components/common';
import type { Channel } from '../types';

function usePersonalHome(source: string, enabled: boolean, revision: string, recent: string) {
    const key = `${source}:${revision}`;
    const [home, setHome] = useState<{ key: string; data: PersonalHome | null }>();
    useEffect(() => {
        if (!enabled) return;
        let request: AbortController | null = null;
        let closed = false;
        const refresh = async () => {
            if (request) return;
            request = new AbortController();
            try {
                const data = await api<PersonalHome>(
                    '/api/library/home',
                    { source },
                    { signal: request.signal },
                );
                if (!closed) setHome({ key, data });
            } catch {
                if (!closed)
                    setHome((previous) => (previous?.key === key ? previous : { key, data: null }));
            } finally {
                request = null;
            }
        };
        void refresh();
        const timer = setInterval(() => void refresh(), 30_000);
        return () => {
            closed = true;
            request?.abort();
            clearInterval(timer);
        };
    }, [source, enabled, key, recent]);
    return { data: home?.key === key ? home.data : null, loading: enabled && home?.key !== key };
}

export function SourceSelector({
    videos = false,
    radio = false,
}: {
    videos?: boolean;
    radio?: boolean;
}) {
    const state = useStore(library);
    const store = videos ? videoSelection : radio ? personalRadioSelection : librarySelection;
    const selection = useStore(store);
    const sources = state.sources.filter(
        (s) =>
            s.enabled &&
            (!videos || s.has_vod || s.report.count > s.report.radio) &&
            (!radio || s.report.radio > 0),
    );
    return (
        <div className="library-source-bar">
            <label>
                Lista{' '}
                <select
                    className="category-select"
                    aria-label={
                        videos
                            ? 'Lista dei video'
                            : radio
                              ? 'Lista delle radio'
                              : 'Lista dei canali'
                    }
                    value={selection.source}
                    onChange={(e) => {
                        store.set({ source: e.target.value, group: '', query: '' });
                        setQuery('');
                    }}
                >
                    {!videos && <option value="builtin">Catalogo Lume</option>}
                    <option value="">Tutte le mie liste</option>
                    {sources.map((source) => (
                        <option key={source.id} value={source.id}>
                            {source.name}
                        </option>
                    ))}
                </select>
            </label>
            <button className="text-button" onClick={() => navigate('playlists')}>
                <Icon name="settings" />
                Gestisci liste
            </button>
        </div>
    );
}
export function PersonalChannelRow({ channel }: { channel: Channel }) {
    const active = playback.store.get().current === channel.id;
    return (
        <div className={`library-channel ${active ? 'is-playing' : ''}`}>
            <button
                className="library-channel-main"
                data-play={channel.id}
                onClick={() => play(channel.id)}
                aria-label={`${channel.content === 'vod' ? 'Riproduci' : channel.kind === 'radio' ? 'Ascolta' : 'Guarda'} ${channel.name}`}
            >
                <StationLogo channel={channel} />
                <span>
                    <strong>{channel.name}</strong>
                    <small>
                        {channel.now?.title ||
                            channel.group ||
                            (channel.content === 'vod' ? 'Video' : 'Diretta')}
                        {active ? ' · In riproduzione' : ''}
                    </small>
                </span>
                <Icon name="play" />
            </button>
            <FavoriteButton channel={channel} className="" />
        </div>
    );
}
export function GroupPicker({
    source,
    content,
    kind,
    selected,
    onSelect,
    onClose,
}: {
    source: string;
    content: string;
    kind: string;
    selected: string;
    onSelect(group: string): void;
    onClose(): void;
}) {
    const [query, setSearch] = useState('');
    return (
        <section className="library-picker" aria-label="Scegli un gruppo">
            <header>
                <h2>Gruppi</h2>
                <button className="text-button" onClick={onClose}>
                    Chiudi
                </button>
            </header>
            <label>
                Cerca un gruppo
                <input
                    type="search"
                    value={query}
                    onChange={(e) => setSearch(e.target.value)}
                    autoFocus
                />
            </label>
            <button className="text-button" onClick={() => onSelect('')}>
                Tutti i gruppi
            </button>
            <ContinuousList<string>
                context={`groups:${source}:${content}:${query}`}
                label="Gruppi della lista"
                height={300}
                rowHeight={52}
                blockSize={50}
                load={(offset, signal) =>
                    api<LibraryPage<string>>(
                        '/api/library/groups',
                        { source, content, kind, query, offset },
                        { signal },
                    )
                }
                render={(group) => (
                    <button
                        className="library-choice"
                        aria-pressed={selected === group}
                        onClick={() => onSelect(group)}
                    >
                        {group}
                    </button>
                )}
            />
        </section>
    );
}
export function LibraryCatalog({
    videos = false,
    radio = false,
    favoritesOnly = false,
    sourceOverride,
    compact = false,
}: {
    videos?: boolean;
    radio?: boolean;
    favoritesOnly?: boolean;
    sourceOverride?: string;
    compact?: boolean;
}) {
    const state = useStore(library);
    const selectionStore = videos
        ? videoSelection
        : radio
          ? personalRadioSelection
          : librarySelection;
    const selection = useStore(selectionStore);
    const [groups, showGroups] = useState(false);
    const source = sourceOverride ?? selection.source;
    const content = videos ? 'vod' : 'live';
    const query = selection.query;
    const favorites = app.get().preferences.favorites;
    const onlyFavorites =
        favoritesOnly ||
        (videos && videoSelection.get().favorites) ||
        (radio && personalRadioSelection.get().favorites);
    const showHome = !compact && !videos && !radio && !onlyFavorites && !query;
    const home = usePersonalHome(
        source,
        showHome && app.get().page !== 'watch',
        state.revision,
        app.get().preferences.recent.join('|'),
    );
    const goToGuide = () => {
        const target =
            source || home.data?.featured?.source_id || state.sources.find((s) => s.enabled)?.id;
        if (target) {
            if (home.data?.featured?.source_id === target)
                writeSession(`guide-channel-${target}`, home.data.featured);
            openGuide(target);
        }
    };
    useEffect(() => {
        if (
            source &&
            source !== 'builtin' &&
            state.ready &&
            !state.sources.some((s) => s.enabled && s.id === source)
        ) {
            selectionStore.set({ source: videos ? '' : 'builtin', group: '' });
        }
    }, [source, state.ready, state.revision, state.sources, selectionStore, videos]);
    const context = `library:${source}:${content}:${onlyFavorites}:${query}:${selection.group}`;
    return (
        <>
            {!compact && (
                <Heading
                    title={
                        videos
                            ? 'I video delle tue liste.'
                            : radio
                              ? 'Le radio delle tue liste.'
                              : favoritesOnly
                                ? 'I tuoi, sempre in prima fila.'
                                : 'La TV, fatta semplice.'
                    }
                    subtitle={
                        videos
                            ? 'Cerca un video e riprendi da dove eri rimasto.'
                            : radio
                              ? 'Scegli una stazione e mettiti all’ascolto.'
                              : 'I canali delle tue liste, pronti da guardare.'
                    }
                />
            )}
            {!compact && videos && <SourceSelector videos />}
            {showHome && !selection.group && (
                <CatalogHero
                    channel={home.data?.featured}
                    loading={home.loading}
                    onGuide={goToGuide}
                />
            )}
            {showHome && <RecentChannels channels={home.data?.recent || []} />}
            {!compact && !videos && !radio && (
                <div className="section-heading">
                    <h2>
                        {onlyFavorites
                            ? 'I tuoi preferiti'
                            : query
                              ? 'Risultati della ricerca'
                              : selection.group || 'Tutti i tuoi canali'}{' '}
                        {!query && !selection.group && home.data && <span>{home.data.total}</span>}
                    </h2>
                    <button className="text-button" onClick={goToGuide}>
                        Apri la guida <Icon name="arrow" />
                    </button>
                </div>
            )}
            <div className="library-filters">
                {(videos || radio) && (
                    <button
                        className="button secondary"
                        aria-pressed={!!onlyFavorites}
                        onClick={() =>
                            (radio ? personalRadioSelection : videoSelection).set({
                                favorites: !onlyFavorites,
                            })
                        }
                    >
                        {radio ? 'Solo preferite' : 'Solo preferiti'}
                    </button>
                )}
                {compact && (
                    <label>
                        Cerca nella lista
                        <input
                            type="search"
                            value={query}
                            onChange={(e) => selectionStore.set({ query: e.target.value })}
                        />
                    </label>
                )}
                {!compact && (
                    <button
                        className="button secondary"
                        aria-expanded={groups}
                        onClick={() => showGroups(!groups)}
                    >
                        {selection.group || 'Tutti i gruppi'}
                        <Icon name="chevron" />
                    </button>
                )}
                {(selection.group || query) && (
                    <>
                        <span className="library-filter-summary">
                            {[selection.group, query && `“${query}”`].filter(Boolean).join(' · ')}
                        </span>
                        <button
                            className="text-button"
                            onClick={() => {
                                selectionStore.set({ group: '', query: '' });
                                setQuery('');
                            }}
                        >
                            Azzera filtri
                        </button>
                    </>
                )}
            </div>
            {groups && (
                <GroupPicker
                    source={source}
                    content={content}
                    kind={radio ? 'radio' : 'tv'}
                    selected={selection.group}
                    onClose={() => showGroups(false)}
                    onSelect={(group) => {
                        selectionStore.set({ group });
                        showGroups(false);
                    }}
                />
            )}
            {state.error && (
                <p className="library-error" role="alert">
                    {state.error}
                </p>
            )}
            <ContinuousList<Channel>
                context={context}
                revision={`${state.revision}:${onlyFavorites ? favorites.join(',') : ''}`}
                refreshInterval={30_000}
                label={videos ? 'Video delle mie liste' : 'Canali delle mie liste'}
                height={compact ? 330 : 520}
                active={app.get().page !== 'watch'}
                layout={compact ? 'list' : radio ? 'radio' : 'channels'}
                itemNames={
                    videos ? ['video', 'video'] : radio ? ['radio', 'radio'] : ['canale', 'canali']
                }
                load={(offset, signal) =>
                    personalCatalog(
                        {
                            source,
                            content,
                            kind: radio ? 'radio' : 'tv',
                            query,
                            group: selection.group,
                            favorites: onlyFavorites,
                            offset,
                            limit: 200,
                        },
                        signal,
                    )
                }
                render={(channel) =>
                    compact ? (
                        <PersonalChannelRow channel={channel} />
                    ) : (
                        <ChannelCard channel={channel} />
                    )
                }
                empty={
                    <EmptyState
                        symbol={videos ? 'play' : 'grid'}
                        title={
                            query || selection.group
                                ? 'Nessun risultato.'
                                : videos
                                  ? 'Nessun video disponibile.'
                                  : 'Nessun canale in questa vista.'
                        }
                        description={
                            query || selection.group
                                ? 'Prova un altro nome o azzera i filtri.'
                                : 'Controlla le liste abilitate e il tipo dei contenuti nelle Impostazioni.'
                        }
                    />
                }
            />
        </>
    );
}

export function PersonalWatchBrowser() {
    useStoreValue(playback.store, (state) => state.mode);
    const current = playback.store.get().channel;
    const [query, setSearch] = useState('');
    const [favorites, setFavorites] = useState(false);
    if (!current?.source_id) return null;
    if (playback.store.get().mode !== 'live')
        return (
            <div className="library-video-actions">
                <h2>Il tuo video</h2>
                <button className="button secondary" onClick={() => playback.restart()}>
                    Dall’inizio
                </button>
                {playback.store.get().mode === 'archive' && (
                    <button className="button primary" onClick={() => playback.goLive()}>
                        Torna alla diretta
                    </button>
                )}
            </div>
        );
    return (
        <>
            <header>
                <h2>{current.kind === 'radio' ? 'Cambia radio' : 'Cambia canale'}</h2>
                <button
                    className="text-button"
                    aria-pressed={favorites}
                    onClick={() => setFavorites(!favorites)}
                >
                    {favorites ? 'Tutti' : 'Solo preferiti'}
                </button>
            </header>
            <input
                className="radio-browser-search"
                type="search"
                placeholder="Cerca nella lista"
                aria-label="Cerca nella lista in riproduzione"
                value={query}
                onChange={(e) => setSearch(e.target.value)}
            />
            <ContinuousList<Channel>
                context={`watch:${current.source_id}:${current.kind}:${query}:${favorites}`}
                revision={favorites ? app.get().preferences.favorites.join(',') : ''}
                label={
                    current.kind === 'radio'
                        ? 'Radio durante l’ascolto'
                        : 'Canali durante la visione'
                }
                height={330}
                blockSize={50}
                refreshInterval={30_000}
                load={(offset, signal) =>
                    personalCatalog(
                        {
                            source: current.source_id,
                            kind: current.kind || 'tv',
                            content: 'live',
                            query,
                            favorites,
                            offset,
                            limit: 50,
                        },
                        signal,
                    )
                }
                render={(channel) => <PersonalChannelRow channel={channel} />}
            />
        </>
    );
}
