import { app, navigate, play, showProgramme } from '../app';
import { progress, range, remainingLabel, selectProgrammes } from '../domain';
import {
    EmptyState,
    FavoriteButton,
    Heading,
    Icon,
    StationLogo,
    stationStyle,
} from '../components/common';
import type { Period } from '../types';

export function Discovery() {
    const state = { ...app.get(), channels: app.get().channels.filter((c) => c.kind !== 'radio') };
    const items = selectProgrammes(state.channels, state.guide.programmes, {
        period: state.period,
        favorites: state.preferences.favorites,
        favoritesOnly: state.favoritesOnly,
        query: state.query,
        now: state.now,
    });
    const selected = items.find((i) => i.key === state.discoveryKey) || items[0];
    const introduction = {
        now: 'In onda adesso. Trova il programma che fa per te.',
        soon: 'Il prossimo momento da non perdere.',
        tonight: 'La serata comincia con una buona scelta.',
    }[state.period];
    return (
        <>
            <Heading title="Cosa guardare." subtitle={introduction} />
            <div data-key="periods" className="discovery-periods">
                <div className="segmented" aria-label="Quando guardare">
                    {(
                        [
                            ['now', 'Ora'],
                            ['soon', 'Tra poco'],
                            ['tonight', 'Stasera'],
                        ] as [Period, string][]
                    ).map(([id, label]) => (
                        <button
                            key={id}
                            data-period={id}
                            aria-pressed={state.period === id}
                            onClick={() => app.set({ period: id, discoveryKey: null })}
                        >
                            {label}
                        </button>
                    ))}
                </div>
                <button
                    className="favorites-filter"
                    data-action="discover-favorites"
                    aria-pressed={state.favoritesOnly}
                    onClick={() => app.set({ favoritesOnly: !state.favoritesOnly })}
                >
                    <Icon name="star" />
                    Solo preferiti
                </button>
            </div>
            {selected ? (
                <div data-key="discovery" className="discovery-layout">
                    <section className="discovery-feature" style={stationStyle(selected.channel)}>
                        <StationLogo channel={selected.channel} />
                        <div className="eyebrow">
                            {selected.programme.start <= state.now
                                ? 'IN ONDA ADESSO'
                                : 'DA NON PERDERE'}{' '}
                            · {selected.channel.name}
                        </div>
                        <h2>{selected.programme.title}</h2>
                        <div className="programme-meta">
                            <span>{range(selected.programme)}</span>
                            <span>{remainingLabel(selected.programme, state.now)}</span>
                        </div>
                        {selected.programme.start <= state.now && (
                            <span className="channel-progress">
                                <i
                                    style={{
                                        width: `${progress(selected.programme, state.now)}%`,
                                        background: 'var(--accent)',
                                    }}
                                />
                            </span>
                        )}
                        <p className="programme-description">
                            {selected.programme.description ||
                                'La descrizione di questo programma non è disponibile nella guida.'}
                        </p>
                        <div className="dialog-actions">
                            <button
                                className="button primary"
                                data-play={selected.channel.id}
                                onClick={() => play(selected.channel.id)}
                            >
                                <Icon name="play" />
                                Guarda {selected.programme.start <= state.now ? 'ora' : 'il canale'}
                            </button>
                            <button
                                className="icon-button"
                                data-programme={selected.key}
                                aria-label="Dettagli del programma"
                                title="Dettagli del programma"
                                onClick={() => showProgramme(selected.key)}
                            >
                                <Icon name="info" />
                            </button>
                            <FavoriteButton channel={selected.channel} className="" />
                        </div>
                    </section>
                    <div data-key="programmes" className="discovery-list" aria-label="Programmi">
                        {items.map((item) => (
                            <button
                                key={item.key}
                                className="discovery-item"
                                data-discover={item.key}
                                data-focus={`discover-${item.key}`}
                                aria-pressed={item.key === selected.key}
                                onClick={() => app.set({ discoveryKey: item.key })}
                            >
                                <StationLogo channel={item.channel} />
                                <span className="item-description">
                                    <strong>{item.programme.title}</strong>
                                    <small>
                                        {item.channel.name} · {range(item.programme)}
                                    </small>
                                </span>
                                <span className="remaining">
                                    {remainingLabel(item.programme, state.now)}
                                </span>
                            </button>
                        ))}
                    </div>
                </div>
            ) : (
                <EmptyState
                    symbol={state.guide.loading ? 'clock' : 'compass'}
                    title={
                        state.guide.loading
                            ? 'Stiamo preparando la tua guida.'
                            : 'Qui c’è spazio per una nuova scoperta.'
                    }
                    description={
                        state.guide.loading
                            ? 'I programmi stanno arrivando. Nel frattempo puoi già guardare i tuoi canali.'
                            : state.guide.error ||
                              'Non ci sono programmi per questi filtri. Prova un altro orario o esplora tutti i canali.'
                    }
                >
                    <button
                        className="button secondary"
                        data-route="channels"
                        onClick={() => navigate('channels')}
                    >
                        Esplora i canali
                    </button>
                </EmptyState>
            )}
        </>
    );
}
