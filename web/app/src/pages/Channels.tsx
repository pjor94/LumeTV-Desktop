import { app, navigate, play, resetFilters } from '../app';
import { currentProgramme, filterChannels, matchesProvider, range } from '../domain';
import {
    BrandIntro,
    ChannelCard,
    EmptyState,
    Heading,
    Icon,
    StationLogo,
} from '../components/common';

function Hero() {
    const state = app.get();
    const channel =
        state.channels.find((c) => c.id === (state.preferences.recent[0] || 'rai1')) ||
        state.channels[0];
    if (!channel) return null;
    const programme = currentProgramme(state.guide.programmes[channel.id], state.now);
    return (
        <section data-key="hero" className="hero" aria-label="Canale in evidenza">
            <BrandIntro>
                <div className="hero-content">
                    <div className="hero-channel">
                        {channel.name}
                        {programme ? ' · In onda adesso' : ' · In diretta'}
                    </div>
                    <h2>{programme?.title || 'Accendi il momento giusto.'}</h2>
                    <p>
                        {programme
                            ? programme.description ||
                              `La diretta di ${channel.name}, da vivere adesso.`
                            : 'I canali che ami, la guida per scegliere. Tutto il piacere della TV, con la semplicità di Lume.'}
                    </p>
                    <div className="hero-actions">
                        <button
                            className="button primary"
                            data-play={channel.id}
                            onClick={() => play(channel.id)}
                        >
                            <Icon name="play" />
                            Guarda {programme ? 'ora' : channel.name}
                        </button>
                        {programme ? (
                            <span className="hero-time">{range(programme)}</span>
                        ) : (
                            <button
                                className="text-button"
                                data-route="discover"
                                style={{ color: '#b7c9e3' }}
                                onClick={() => navigate('discover')}
                            >
                                Cosa guardare <Icon name="arrow" />
                            </button>
                        )}
                    </div>
                </div>
                <StationLogo channel={channel} extra="hero-station" />
            </BrandIntro>
        </section>
    );
}
export function Channels({ favoritesOnly = false }: { favoritesOnly?: boolean }) {
    const state = app.get();
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
                state.group === 'all' && <Hero key="hero" />}
            {!favoritesOnly && !state.query && !!recent.length && (
                <section
                    key="recent"
                    data-key="recent"
                    className="recent-strip"
                    aria-label="Visti di recente"
                >
                    <span className="recent-label">Visti di recente</span>
                    <div className="recent-row">
                        {recent.slice(0, 5).map((c) => (
                            <button
                                key={c.id}
                                className="recent-channel"
                                data-play={c.id}
                                onClick={() => play(c.id)}
                            >
                                <StationLogo channel={c} />
                                <span>
                                    <strong>{c.name}</strong>
                                    <small>
                                        {currentProgramme(state.guide.programmes[c.id], state.now)
                                            ?.title || 'Torna alla diretta'}
                                    </small>
                                </span>
                            </button>
                        ))}
                    </div>
                </section>
            )}
            <div key="channel-heading" data-key="channel-heading" className="section-heading">
                <h2>
                    {favoritesOnly ? 'I tuoi preferiti' : 'Tutti i tuoi canali'}{' '}
                    <span>{channels.length}</span>
                </h2>
                {!favoritesOnly && (
                    <button
                        className="text-button"
                        data-route="guide"
                        onClick={() => navigate('guide')}
                    >
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
