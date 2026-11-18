import { app, navigate, play } from '../app';
import { channelProgramme, range } from '../domain';
import { BrandIntro, Icon, StationLogo } from './common';
import type { Channel } from '../types';

export function CatalogHero({
    channel,
    loading = false,
    onGuide,
}: {
    channel?: Channel | null;
    loading?: boolean;
    onGuide?: () => void;
}) {
    if (!channel)
        return loading ? (
            <section
                className="hero catalog-hero-loading"
                aria-label="Caricamento del canale in evidenza"
                aria-busy="true"
            >
                <div className="hero-content">
                    <div className="hero-channel">Caricamento della tua lista…</div>
                    <h2>Accendi il momento giusto.</h2>
                    <p>
                        I canali che ami, la guida per scegliere. Tutto il piacere della TV, con la
                        semplicità di Lume.
                    </p>
                    <div className="hero-actions">
                        <button className="button primary" disabled>
                            <Icon name="play" />
                            Guarda ora
                        </button>
                    </div>
                </div>
            </section>
        ) : null;
    const state = app.get();
    const programme = channelProgramme(channel, state.guide.programmes, state.now);
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
                            aria-label={`Guarda ora ${channel.name}`}
                            onClick={() => play(channel.id)}
                        >
                            <Icon name="play" />
                            Guarda ora
                        </button>
                        {programme ? (
                            <span className="hero-time">{range(programme)}</span>
                        ) : (
                            <button
                                className="text-button"
                                data-route={onGuide ? undefined : 'discover'}
                                style={{ color: '#b7c9e3' }}
                                onClick={onGuide || (() => navigate('discover'))}
                            >
                                {onGuide ? 'Apri la guida' : 'Cosa guardare'} <Icon name="arrow" />
                            </button>
                        )}
                    </div>
                </div>
                <StationLogo channel={channel} extra="hero-station" />
            </BrandIntro>
        </section>
    );
}

export function RecentChannels({ channels }: { channels: Channel[] }) {
    const state = app.get();
    if (!channels.length) return null;
    return (
        <section data-key="recent" className="recent-strip" aria-label="Visti di recente">
            <span className="recent-label">Visti di recente</span>
            <div className="recent-row">
                {channels.slice(0, 5).map((channel) => {
                    const programme = channelProgramme(channel, state.guide.programmes, state.now);
                    return (
                        <button
                            key={channel.id}
                            className="recent-channel"
                            data-play={channel.id}
                            onClick={() => play(channel.id)}
                        >
                            <StationLogo channel={channel} />
                            <span>
                                <strong>{channel.name}</strong>
                                <small>{programme?.title || 'Torna alla diretta'}</small>
                            </span>
                        </button>
                    );
                })}
            </div>
        </section>
    );
}
