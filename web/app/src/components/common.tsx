import { useLayoutEffect, useRef, type CSSProperties, type ReactNode } from 'react';
import { app, playback, play, savePreferences } from '../app';
import { currentProgramme, date, progress, providerName } from '../domain';
import { motion } from '../motion';
import type { Channel } from '../types';

export const stationStyle = (channel: Channel) => ({ '--station': channel.color }) as CSSProperties;
export function Icon({ name, className = '' }: { name: string; className?: string }) {
    return (
        <svg className={className} aria-hidden="true">
            <use href={`#i-${name}`} />
        </svg>
    );
}
export function StationLogo({ channel, extra = '' }: { channel: Channel; extra?: string }) {
    const state = app.get();
    const name = channel.name.replace(/^Rai\s*/, '');
    const url = state.guide.logos[channel.id]
        ? `/art/channel/${encodeURIComponent(channel.id)}?v=${state.guide.updated_at}`
        : null;
    return (
        <span className={`station-logo ${extra}`} style={stationStyle(channel)} aria-hidden="true">
            {channel.provider === 'rai' && /^[12345]$/.test(name) ? (
                <span className="rai-word">
                    Rai<b>{name}</b>
                </span>
            ) : (
                <span className={`station-fallback ${channel.name.length > 7 ? 'small' : ''}`}>
                    {channel.name}
                </span>
            )}
            {url && (
                <img
                    key={url}
                    src={url}
                    alt=""
                    loading="lazy"
                    decoding="async"
                    onLoad={(event) => {
                        event.currentTarget.dataset.loaded = '';
                    }}
                    onError={(event) => {
                        event.currentTarget.hidden = true;
                    }}
                />
            )}
        </span>
    );
}
export function FavoriteButton({
    channel,
    className = 'favorite-button',
}: {
    channel: Channel;
    className?: string;
}) {
    const favorite = app.get().preferences.favorites.includes(channel.id);
    return (
        <button
            className={`icon-button ${className}`}
            data-favorite={channel.id}
            aria-pressed={favorite}
            aria-label={`${favorite ? 'Rimuovi' : 'Aggiungi'} ${channel.name} ${favorite ? 'dai' : 'ai'} preferiti`}
            title={favorite ? 'Rimuovi dai preferiti' : 'Aggiungi ai preferiti'}
            onClick={async (event) => {
                const button = event.currentTarget;
                const card = button.closest('.channel-card');
                const neighbour =
                    card?.nextElementSibling?.querySelector<HTMLButtonElement>('[data-favorite]') ||
                    card?.previousElementSibling?.querySelector<HTMLButtonElement>(
                        '[data-favorite]',
                    );
                const restore =
                    document.documentElement.dataset.input === 'keyboard' &&
                    document.activeElement === button;
                await savePreferences(
                    { toggle_favorite: channel.id },
                    favorite ? 'Rimosso dai preferiti.' : 'Aggiunto ai tuoi preferiti.',
                );
                if (button.isConnected) motion.confirmFavorite(button);
                else if (
                    restore &&
                    document.activeElement === document.body &&
                    app.get().page === 'favorites'
                )
                    (neighbour?.isConnected
                        ? neighbour
                        : document.querySelector<HTMLButtonElement>('.empty-state button')
                    )?.focus({ preventScroll: true });
            }}
        >
            <Icon name="star" />
        </button>
    );
}
export function ChannelCard({ channel }: { channel: Channel }) {
    const state = app.get();
    const programme = currentProgramme(state.guide.programmes[channel.id], state.now);
    return (
        <article
            data-key={`channel-${channel.id}`}
            className={`channel-card ${playback.store.get().current === channel.id ? 'is-playing' : ''}`}
            style={stationStyle(channel)}
        >
            <button
                className="channel-main"
                data-play={channel.id}
                data-focus={`channel-${channel.id}`}
                aria-label={`Guarda ${channel.name}${programme ? ', ' + programme.title : ''}`}
                onClick={() => play(channel.id)}
            >
                <span className="channel-number">
                    {String(channel.number).padStart(2, '0')} <span>/</span>{' '}
                    {providerName(channel.provider)}
                </span>
                <StationLogo channel={channel} />
                <span className="channel-name">{channel.name}</span>
                <span className="channel-programme">{programme?.title || 'Guarda in diretta'}</span>
                {programme && (
                    <span className="channel-progress">
                        <i style={{ width: `${progress(programme, state.now)}%` }} />
                    </span>
                )}
                <span className="channel-play-hint">
                    <Icon name="play" />
                </span>
            </button>
            <FavoriteButton channel={channel} />
        </article>
    );
}
export function Heading({ title, subtitle }: { title: string; subtitle: string }) {
    return (
        <div data-key="heading" className="page-heading">
            <div>
                <h1>{title}</h1>
                <p>{subtitle}</p>
            </div>
            <span className="heading-date">{date(app.get().now)}</span>
        </div>
    );
}
export function EmptyState({
    symbol,
    title,
    description,
    children,
}: {
    symbol: string;
    title: string;
    description: string;
    children?: ReactNode;
}) {
    return (
        <div data-key="empty" className="empty-state">
            <div className="empty-symbol">
                <Icon name={symbol} />
            </div>
            <h2>{title}</h2>
            <p>{description}</p>
            {children}
        </div>
    );
}
export function Equalizer() {
    return (
        <span className="equalizer">
            <i />
            <i />
            <i />
        </span>
    );
}
export function BrandIntro({ children }: { children: ReactNode }) {
    const ref = useRef<HTMLImageElement>(null);
    useLayoutEffect(() => {
        motion.introduceBrand(ref.current);
    }, []);
    return (
        <>
            {children}
            <img ref={ref} className="hero-mark" src="/assets/symbol.svg" alt="" />
        </>
    );
}
