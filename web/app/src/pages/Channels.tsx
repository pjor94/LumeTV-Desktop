import { personalizedChannels } from '../domain';
import { library, librarySelection } from '../personal';
import { useStore } from '../store';
import { LibraryCatalog, SourceSelector } from './LibraryCatalog';
import { app, openGuide, resetFilters } from '../app';
import { filterChannels, matchesProvider } from '../domain';
import { ChannelCard, EmptyState, Heading, Icon } from '../components/common';
import { CatalogHero, RecentChannels } from '../components/CatalogHome';
export function BuiltinChannels({ favoritesOnly = false }: { favoritesOnly?: boolean }) {
    const state = {
        ...app.get(),
        channels: personalizedChannels(app.get().channels, app.get().preferences).filter(
            (c) => c.kind !== 'radio',
        ),
    };
    const channels = filterChannels(state.channels, {
        provider: state.provider,
        group: state.group,
        favoritesOnly,
        favorites: state.preferences.favorites,
        query: state.query,
        programmes: state.guide.programmes,
        now: state.now,
    });
    const recent = state.preferences.recent.flatMap((id) => {
        const c = state.channels.find((c) => c.id === id);
        return c ? [c] : [];
    });
    const groups = Array.from(new Set(state.channels.map((c) => c.group)));
    return (
        <>
            <Heading
                title={favoritesOnly ? 'I tuoi, sempre in prima fila.' : 'La TV, fatta semplice.'}
                subtitle={
                    favoritesOnly
                        ? 'I canali che ami. Un piccolo gesto, tutto il tuo mondo.'
                        : `${state.channels.length} canali. Scegli il tuo, al resto pensa Lume.`
                }
            />
            {!favoritesOnly &&
                !state.query &&
                state.provider === 'all' &&
                state.group === 'all' && <CatalogHero channel={recent[0] || state.channels[0]} />}
            {!favoritesOnly && !state.query && !!recent.length && (
                <RecentChannels channels={recent} />
            )}
            <div key="channel-heading" data-key="channel-heading" className="section-heading">
                <h2>
                    {favoritesOnly ? 'I tuoi preferiti' : 'Tutti i tuoi canali'}{' '}
                    <span>{channels.length}</span>
                </h2>
                {!favoritesOnly && (
                    <button className="text-button" data-route="guide" onClick={() => openGuide()}>
                        Apri la guida <Icon name="arrow" />
                    </button>
                )}
            </div>
            <div key="filters" data-key="filters" className="filter-bar">
                <div className="segmented provider-options" aria-label="Filtra per emittente">
                    {[
                        ['all', 'Tutti'],
                        ['rai', 'Rai'],
                        ['mediaset', 'Mediaset'],
                        ['la7', 'La7'],
                        ['sky', 'Sky'],
                        ['discovery', 'Discovery'],
                        ['direct', 'Altri'],
                    ]
                        .filter(
                            ([id]) =>
                                id === 'all' || state.channels.some((c) => matchesProvider(c, id)),
                        )
                        .map(([id, name]) => (
                            <button
                                key={id}
                                data-provider={id}
                                aria-pressed={state.provider === id}
                                onClick={() => app.set({ provider: id })}
                            >
                                {name}
                            </button>
                        ))}
                </div>
                <select
                    className="category-select"
                    id="group-filter"
                    aria-label="Categoria"
                    value={state.group}
                    onChange={(event) => app.set({ group: event.target.value })}
                >
                    <option value="all">Tutte le categorie</option>
                    {groups.map((group) => (
                        <option key={group} value={group}>
                            {group}
                        </option>
                    ))}
                </select>
                <span className="filter-count">
                    {channels.length} {channels.length === 1 ? 'canale' : 'canali'}
                </span>
            </div>
            {channels.length ? (
                <div key="channels" data-key="channels" className="channel-grid">
                    {channels.map((channel) => (
                        <ChannelCard key={channel.id} channel={channel} />
                    ))}
                </div>
            ) : (
                <EmptyState
                    symbol={favoritesOnly ? 'star' : 'search'}
                    title={
                        favoritesOnly && !state.query
                            ? 'Fai spazio ai tuoi preferiti.'
                            : 'Nessun canale trovato.'
                    }
                    description={
                        favoritesOnly && !state.query
                            ? 'Premi la stella su una scheda: ritroverai qui i tuoi canali, pronti da guardare.'
                            : 'Prova un altro nome o modifica i filtri.'
                    }
                >
                    <button
                        className="button secondary"
                        data-action="reset-filters"
                        onClick={resetFilters}
                    >
                        {favoritesOnly ? 'Esplora i canali' : 'Mostra tutti i canali'}
                    </button>
                </EmptyState>
            )}
        </>
    );
}

export function Channels({ favoritesOnly = false }: { favoritesOnly?: boolean }) {
    const personal = useStore(library);
    const selection = useStore(librarySelection);
    return (
        <>
            {personal.sources.some((s) => s.enabled) && <SourceSelector />}
            {selection.source !== 'builtin' && personal.sources.some((s) => s.enabled) ? (
                <LibraryCatalog favoritesOnly={favoritesOnly} />
            ) : (
                <BuiltinChannels favoritesOnly={favoritesOnly} />
            )}
        </>
    );
}
