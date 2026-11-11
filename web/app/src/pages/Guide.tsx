import { app, play, showProgramme } from '../app';
import { date, filterChannels, range, schedule, time } from '../domain';
import { EmptyState, Heading, Icon, StationLogo } from '../components/common';
import type { CSSProperties } from 'react';

function guideTime(period: 'now' | 'tonight') {
    const start = new Date();
    if (period === 'tonight') start.setHours(20, 0, 0, 0);
    app.set({ guideStart: Math.floor(start.getTime() / 1_800_000) * 1_800_000 });
}
function shift(direction: number) {
    const guideStart = Math.min(
        Date.now() + 44 * 3_600_000,
        Math.max(Date.now() - 6 * 3_600_000, app.get().guideStart + direction * 4 * 3_600_000),
    );
    app.set({ guideStart: Math.floor(guideStart / 1_800_000) * 1_800_000 });
}
export function GuidePage() {
    const state = app.get();
    const channels = filterChannels(state.channels, {
        favoritesOnly: state.favoritesOnly,
        favorites: state.preferences.favorites,
        query: state.query,
        programmes: state.guide.programmes,
        now: state.now,
    });
    const start = state.guideStart,
        end = start + 4 * 3_600_000,
        width = 912;
    const x = (value: number) => ((value - start) / 60_000) * 3.8;
    const nowX = x(state.now);
    const trackStyle = {
        width,
        ...(nowX >= 0 && nowX <= width ? { '--now-x': `${nowX}px` } : {}),
    } as CSSProperties;
    return (
        <>
            <Heading
                title="La tua serata, a colpo d’occhio."
                subtitle="Scorri la guida. Scegli un programma. Trova il tuo momento."
            />
            <div data-key="guide-toolbar" className="guide-toolbar">
                <div className="segmented" aria-label="Canali nella guida">
                    <button
                        data-guide-favorites="false"
                        aria-pressed={!state.favoritesOnly}
                        onClick={() => app.set({ favoritesOnly: false })}
                    >
                        Tutti
                    </button>
                    <button
                        data-guide-favorites="true"
                        aria-pressed={state.favoritesOnly}
                        onClick={() => app.set({ favoritesOnly: true })}
                    >
                        Preferiti
                    </button>
                </div>
                <span className="guide-spacer" />
                <span className="guide-date">{date(start)}</span>
                <button
                    className="button secondary"
                    data-guide-time="now"
                    onClick={() => guideTime('now')}
                >
                    Adesso
                </button>
                <button
                    className="button secondary"
                    data-guide-time="tonight"
                    onClick={() => guideTime('tonight')}
                >
                    Stasera
                </button>
                <button
                    className="icon-button"
                    data-guide-shift="-1"
                    aria-label="Quattro ore prima"
                    title="Quattro ore prima"
                    onClick={() => shift(-1)}
                >
                    <Icon name="chevron" className="reverse-chevron" />
                </button>
                <button
                    className="icon-button"
                    data-guide-shift="1"
                    aria-label="Quattro ore dopo"
                    title="Quattro ore dopo"
                    onClick={() => shift(1)}
                >
                    <Icon name="chevron" />
                </button>
            </div>
            {channels.length ? (
                <div className="guide-scroll" id="guide-scroll">
                    <div className="guide-row guide-header">
                        <span className="guide-channel">CANALI</span>
                        <div className="guide-times" style={trackStyle}>
                            {Array.from({ length: 8 }, (_, i) => (
                                <span key={i} style={{ left: i * 114 }}>
                                    {time(start + i * 1_800_000)}
                                </span>
                            ))}
                        </div>
                    </div>
                    {channels.map((channel) => {
                        const shows = schedule(state.guide.programmes[channel.id]).filter(
                            (p) => p.end > start && p.start < end,
                        );
                        return (
                            <div
                                key={channel.id}
                                data-key={`guide-${channel.id}`}
                                className="guide-row"
                            >
                                <button
                                    className="guide-channel"
                                    data-play={channel.id}
                                    aria-label={`Guarda ${channel.name}`}
                                    onClick={() => play(channel.id)}
                                >
                                    <span className="guide-channel-number">
                                        {String(channel.number).padStart(2, '0')}
                                    </span>
                                    <StationLogo channel={channel} />
                                    <strong>{channel.name}</strong>
                                </button>
                                <div className="programme-track" style={trackStyle}>
                                    {shows.length ? (
                                        shows.map((p) => {
                                            const left = Math.max(0, x(p.start)),
                                                right = Math.min(width, x(p.end));
                                            return (
                                                <button
                                                    key={`${channel.id}~${p.start}`}
                                                    className={`programme-block ${p.start <= state.now && p.end > state.now ? 'live' : ''}`}
                                                    data-programme={`${channel.id}~${p.start}`}
                                                    data-focus={`guide-${channel.id}-${p.start}`}
                                                    style={{
                                                        left: left + 3,
                                                        width: Math.max(3, right - left - 6),
                                                    }}
                                                    aria-label={`${p.title}, ${channel.name}, ${range(p)}`}
                                                    onClick={() =>
                                                        showProgramme(`${channel.id}~${p.start}`)
                                                    }
                                                >
                                                    <strong>{p.title}</strong>
                                                    <small>{range(p)}</small>
                                                </button>
                                            );
                                        })
                                    ) : (
                                        <span className="programme-gap">
                                            {state.guide.loading
                                                ? 'Caricamento della guida…'
                                                : 'Programmazione non disponibile per questo orario'}
                                        </span>
                                    )}
                                </div>
                            </div>
                        );
                    })}
                </div>
            ) : (
                <EmptyState
                    symbol="guide"
                    title="Nessun canale in questa guida."
                    description="Aggiungi un preferito o modifica i filtri per ritrovare i tuoi programmi."
                />
            )}
            {state.guide.error && <p className="notice">{state.guide.error}</p>}
        </>
    );
}
