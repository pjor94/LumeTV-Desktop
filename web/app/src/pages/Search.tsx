import { app, showProgramme } from '../app';
import { date, filterChannels, range, schedule, searchable, time } from '../domain';
import { ChannelCard, EmptyState, Heading, StationLogo } from '../components/common';

export function Search() {
    const state = app.get();
    const channels = filterChannels(state.channels, {
        query: state.query,
        programmes: state.guide.programmes,
        now: state.now,
    });
    const words = searchable(state.query).split(' ').filter(Boolean);
    const programmes = state.channels
        .flatMap((channel) =>
            schedule(state.guide.programmes[channel.id])
                .filter(
                    (p) =>
                        p.end > state.now &&
                        words.every((word) =>
                            searchable(
                                `${p.title} ${channel.name} ${p.description || ''}`,
                            ).includes(word),
                        ),
                )
                .slice(0, 3)
                .map((programme) => ({ channel, programme })),
        )
        .slice(0, 30);
    return (
        <>
            <Heading title="Un momento da trovare." subtitle={`Risultati per “${state.query}”`} />
            {!!channels.length && (
                <>
                    <div className="section-heading">
                        <h2>
                            Canali <span>{channels.length}</span>
                        </h2>
                    </div>
                    <div data-key="channels" className="channel-grid">
                        {channels.map((channel) => (
                            <ChannelCard key={channel.id} channel={channel} />
                        ))}
                    </div>
                </>
            )}
            {!!programmes.length && (
                <>
                    <div className="section-heading">
                        <h2>
                            Programmi <span>{programmes.length}</span>
                        </h2>
                    </div>
                    <div data-key="programmes" className="discovery-list">
                        {programmes.map(({ channel, programme: p }) => (
                            <button
                                key={`${channel.id}~${p.start}`}
                                className="discovery-item"
                                data-programme={`${channel.id}~${p.start}`}
                                onClick={() => showProgramme(`${channel.id}~${p.start}`)}
                            >
                                <StationLogo channel={channel} />
                                <span className="item-description">
                                    <strong>{p.title}</strong>
                                    <small>
                                        {channel.name} · {date(p.start)} · {range(p)}
                                    </small>
                                </span>
                                <span className="remaining">
                                    {p.start <= state.now ? 'In onda' : time(p.start)}
                                </span>
                            </button>
                        ))}
                    </div>
                </>
            )}
            {!channels.length && !programmes.length && (
                <EmptyState
                    symbol="search"
                    title="Questo titolo si fa desiderare."
                    description="Prova il nome di un canale, di un programma o una parola diversa."
                />
            )}
        </>
    );
}
