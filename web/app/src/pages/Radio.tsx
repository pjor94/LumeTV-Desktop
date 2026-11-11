import { app, refreshCatalog } from '../app';
import { filterChannels, isRadio } from '../domain';
import { EmptyState, Heading, Icon, RadioCard } from '../components/common';
import '../radio.css';

const PAGE_SIZE = 48;
export function Radio() {
    const state = app.get();
    const filter = state.radioFilter;
    const setFilter = (radioFilter: typeof filter) => app.set({ radioFilter, radioPage: 0 });
    const radio = state.catalog.radio;
    const all = state.channels.filter(isRadio);
    const ordered =
        filter === 'recent'
            ? state.preferences.recent.map((id) => all.find((c) => c.id === id)).filter((c) => !!c)
            : all;
    const matches = filterChannels(ordered, {
        kind: 'radio',
        query: state.query,
        favoritesOnly: filter === 'favorites',
        favorites: state.preferences.favorites,
    });
    const page = Math.min(state.radioPage, Math.max(0, Math.ceil(matches.length / PAGE_SIZE) - 1));
    const start = page * PAGE_SIZE;
    const changePage = (page: number) => {
        app.set({ radioPage: page });
        document.getElementById('page-content')?.scrollIntoView({ block: 'start' });
    };
    return (
        <>
            <Heading
                title="Radio, in diretta."
                subtitle="Le radio italiane di WorldRadio. Cerca per nome o città."
            />
            <div className="radio-toolbar">
                <div className="segmented" aria-label="Filtra le radio">
                    {(
                        [
                            ['all', 'Tutte'],
                            ['favorites', 'Preferite'],
                            ['recent', 'Recenti'],
                        ] as const
                    ).map(([id, label]) => (
                        <button key={id} aria-pressed={filter === id} onClick={() => setFilter(id)}>
                            {label}
                        </button>
                    ))}
                </div>
                <button
                    className="text-button"
                    disabled={!!radio?.loading || (radio?.next_retry_at || 0) > state.now}
                    onClick={() => void refreshCatalog(true, true)}
                >
                    <Icon name="refresh" />
                    {radio?.loading ? 'Aggiorno le radio…' : 'Aggiorna radio'}
                </button>
            </div>
            {radio?.error && (
                <p className="radio-notice" role="status">
                    {radio.error}
                </p>
            )}
            {matches.length ? (
                <>
                    <div className="radio-results" role="status">
                        {matches.length.toLocaleString('it-IT')} radio
                        {state.query && ` per “${state.query}”`}
                    </div>
                    <div className="radio-grid">
                        {matches.slice(start, start + PAGE_SIZE).map((channel) => (
                            <RadioCard key={channel.id} channel={channel} />
                        ))}
                    </div>
                    {matches.length > PAGE_SIZE && (
                        <nav className="radio-pagination" aria-label="Pagine delle radio">
                            <button
                                className="text-button"
                                disabled={page === 0}
                                onClick={() => changePage(page - 1)}
                            >
                                Precedenti
                            </button>
                            <span>
                                {start + 1}–{Math.min(start + PAGE_SIZE, matches.length)} di{' '}
                                {matches.length.toLocaleString('it-IT')}
                            </span>
                            <button
                                className="text-button"
                                disabled={start + PAGE_SIZE >= matches.length}
                                onClick={() => changePage(page + 1)}
                            >
                                Successive
                            </button>
                        </nav>
                    )}
                </>
            ) : (
                <EmptyState
                    symbol="radio"
                    title={
                        filter === 'favorites'
                            ? 'Le tue radio preferite.'
                            : filter === 'recent'
                              ? 'Nessuna radio ascoltata.'
                              : 'Nessuna radio trovata.'
                    }
                    description={
                        filter === 'favorites'
                            ? 'Premi la stella accanto a una stazione per ritrovarla qui.'
                            : filter === 'recent'
                              ? 'Le radio ascoltate compariranno in questa lista.'
                              : 'Prova un altro nome o una città.'
                    }
                />
            )}
        </>
    );
}
