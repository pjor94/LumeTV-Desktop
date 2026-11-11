import { app, playback, play, showProgramme, navigate } from '../app';
import {
    currentProgramme,
    providerName,
    range,
    remainingLabel,
    schedule,
    isRadio,
    filterChannels,
} from '../domain';
import { useState } from 'react';
import { useStoreValue } from '../store';
import { Equalizer, FavoriteButton, Icon, StationLogo } from './common';

export function WatchDetails() {
    const nowPlaying = useStoreValue(playback.store, (s) => s.nowPlaying);
    const state = app.get();
    const channel = playback.store.get().channel;
    if (!channel) return null;
    if (isRadio(channel))
        return (
            <>
                <div className="watch-heading">
                    <StationLogo channel={channel} />
                    <div>
                        <span className="watch-station-name">{channel.name}</span>
                        <small>Radio · {channel.group}</small>
                    </div>
                    <FavoriteButton channel={channel} className="" />
                </div>
                <h1>{nowPlaying || `In ascolto di ${channel.name}`}</h1>
                <p className="programme-description">
                    {nowPlaying
                        ? 'Titolo comunicato dall’emittente.'
                        : 'La stazione non sta comunicando il titolo del brano.'}
                </p>
            </>
        );
    const now = currentProgramme(state.guide.programmes[channel.id], state.now);
    const next = schedule(state.guide.programmes[channel.id]).find((p) => p.start > state.now);
    return (
        <>
            <div className="watch-heading">
                <StationLogo channel={channel} />
                <div>
                    <span className="watch-station-name">{channel.name}</span>
                    <small>
                        Canale {channel.number} · {providerName(channel.provider)}
                    </small>
                </div>
                <FavoriteButton channel={channel} className="" />
            </div>
            <div className="eyebrow">IN ONDA ADESSO</div>
            <h1>{now?.title || `La diretta di ${channel.name}`}</h1>
            {now ? (
                <>
                    <div className="programme-meta">
                        <span>{range(now)}</span>
                        <span>{remainingLabel(now, state.now)}</span>
                    </div>
                    <p className="programme-description">{now.description || ''}</p>
                </>
            ) : (
                <p className="programme-description">
                    La programmazione di questo canale non è al momento disponibile nella guida.
                </p>
            )}
            {next && (
                <div className="watch-next">
                    <span>A SEGUIRE</span>
                    <div>
                        <strong>{next.title}</strong>
                        <small>{range(next)}</small>
                    </div>
                    <button
                        className="text-button"
                        data-programme={`${channel.id}~${next.start}`}
                        aria-label="Dettagli del prossimo programma"
                        onClick={() => showProgramme(`${channel.id}~${next.start}`)}
                    >
                        <Icon name="info" />
                    </button>
                </div>
            )}
        </>
    );
}
export function WatchBrowser() {
    if (isRadio(playback.store.get().channel)) return <RadioBrowser />;
    const state = app.get();
    const current = playback.store.get().current;
    const channels = state.watchFavorites
        ? state.channels.filter((c) => !isRadio(c) && state.preferences.favorites.includes(c.id))
        : state.channels.filter((c) => !isRadio(c));
    return (
        <>
            <header>
                <h2>Cambia canale</h2>
                <div className="segmented">
                    <button
                        data-watch-filter="all"
                        aria-pressed={!state.watchFavorites}
                        onClick={() => app.set({ watchFavorites: false })}
                    >
                        Tutti
                    </button>
                    <button
                        data-watch-filter="favorites"
                        aria-pressed={state.watchFavorites}
                        onClick={() => app.set({ watchFavorites: true })}
                    >
                        Preferiti
                    </button>
                </div>
            </header>
            <div className="watch-channel-list" id="watch-channel-list">
                {channels.length ? (
                    channels.map((c) => (
                        <button
                            key={c.id}
                            className="watch-channel"
                            data-play={c.id}
                            aria-pressed={current === c.id}
                            aria-label={`Guarda ${c.name}`}
                            onClick={() => play(c.id)}
                        >
                            <span>{String(c.number).padStart(2, '0')}</span>
                            <StationLogo channel={c} />
                            <span className="watch-channel-description">
                                <strong>{c.name}</strong>
                                <small>
                                    {currentProgramme(state.guide.programmes[c.id], state.now)
                                        ?.title || 'Guarda in diretta'}
                                </small>
                            </span>
                            {current === c.id && <Equalizer />}
                        </button>
                    ))
                ) : (
                    <p className="notice">Aggiungi i tuoi canali preferiti con la stella.</p>
                )}
            </div>
        </>
    );
}
function RadioBrowser() {
    const [query, setQuery] = useState('');
    const state = app.get();
    const channels = filterChannels(state.channels, {
        kind: 'radio',
        query,
        favoritesOnly: state.watchFavorites,
        favorites: state.preferences.favorites,
    });
    const current = playback.store.get().current;
    return (
        <>
            <header>
                <h2>Cambia radio</h2>
                <div className="segmented">
                    <button
                        aria-pressed={!state.watchFavorites}
                        onClick={() => app.set({ watchFavorites: false })}
                    >
                        Tutte
                    </button>
                    <button
                        aria-pressed={state.watchFavorites}
                        onClick={() => app.set({ watchFavorites: true })}
                    >
                        Preferite
                    </button>
                </div>
            </header>
            <input
                className="radio-browser-search"
                type="search"
                aria-label="Cerca tra le radio"
                placeholder="Nome o città"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
            />
            <div className="watch-channel-list" id="watch-channel-list">
                {channels.slice(0, 60).map((c) => (
                    <button
                        className="watch-channel radio-watch-channel"
                        key={c.id}
                        data-play={c.id}
                        aria-pressed={current === c.id}
                        aria-label={`Ascolta ${c.name}`}
                        onClick={() => play(c.id)}
                    >
                        <StationLogo channel={c} />
                        <span className="watch-channel-description">
                            <strong>{c.name}</strong>
                            <small>{c.group}</small>
                        </span>
                        {current === c.id && <Equalizer />}
                    </button>
                ))}
                {!channels.length && <p className="radio-browser-note">Nessuna radio trovata.</p>}
                {channels.length > 60 && (
                    <button
                        className="text-button radio-browser-note"
                        onClick={() => navigate('radio')}
                    >
                        Apri il catalogo delle {channels.length.toLocaleString('it-IT')} radio
                    </button>
                )}
            </div>
        </>
    );
}
