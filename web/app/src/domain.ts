import type { Channel, Period, Programme } from './types';
export const isRadio = (channel: Channel | null | undefined) => channel?.kind === 'radio';

export const searchable = (value: unknown) =>
    String(value || '')
        .normalize('NFD')
        .replace(/\p{M}/gu, '')
        .toLowerCase()
        .replace(/[^\p{L}\p{N}]+/gu, ' ')
        .trim();
export function schedule(programmes: Programme[] = []) {
    const sorted = programmes
        .filter(
            (p) =>
                p &&
                Number.isFinite(p.start) &&
                Number.isFinite(p.end) &&
                p.end > p.start &&
                !/^EPG\s+(non disponibile|not available)$/i.test(String(p.title || '').trim()),
        )
        .slice()
        .sort((a, b) => a.start - b.start);
    return sorted
        .map((p, index) => ({ ...p, end: Math.min(p.end, sorted[index + 1]?.start || p.end) }))
        .filter((p) => p.end > p.start);
}
export const currentProgramme = (programmes?: Programme[], now = Date.now()) =>
    schedule(programmes).find((p) => p.start <= now && p.end > now) || null;
export const progress = (p: Programme | null, now = Date.now()) =>
    p && p.end > p.start
        ? Math.max(0, Math.min(100, ((now - p.start) / (p.end - p.start)) * 100))
        : 0;
export const matchesProvider = (channel: Channel, provider: string) =>
    provider === 'all' ||
    channel.provider === provider ||
    (provider === 'rai' && channel.provider === 'rainewsrg');
interface Filters {
    kind?: 'tv' | 'radio';
    provider?: string;
    group?: string;
    favoritesOnly?: boolean;
    favorites?: string[];
    query?: string;
    programmes?: Record<string, Programme[]>;
    now?: number;
}
export function filterChannels(
    channels: Channel[],
    {
        kind = 'tv',
        provider = 'all',
        group = 'all',
        favoritesOnly = false,
        favorites = [],
        query = '',
        programmes = {},
        now = Date.now(),
    }: Filters = {},
) {
    const words = searchable(query).split(' ').filter(Boolean);
    return channels.filter(
        (c) =>
            isRadio(c) === (kind === 'radio') &&
            matchesProvider(c, provider) &&
            (group === 'all' || c.group === group) &&
            (!favoritesOnly || favorites.includes(c.id)) &&
            words.every((word) =>
                searchable(
                    `${c.name} ${c.number} ${c.group} ${currentProgramme(programmes[c.id], now)?.title || ''}`,
                ).includes(word),
            ),
    );
}
export function selectProgrammes(
    channels: Channel[],
    programmes: Record<string, Programme[]>,
    {
        period = 'now',
        favorites = [],
        favoritesOnly = false,
        query = '',
        now = Date.now(),
    }: Filters & { period?: Period } = {},
) {
    const words = searchable(query).split(' ').filter(Boolean);
    const evening = new Date(now);
    evening.setHours(20, 30, 0, 0);
    const midnight = new Date(now);
    midnight.setHours(24, 0, 0, 0);
    return channels
        .filter((c) => !favoritesOnly || favorites.includes(c.id))
        .flatMap((channel) => {
            const shows = schedule(programmes[channel.id]);
            const candidates =
                period === 'tonight'
                    ? shows
                          .filter(
                              (p) =>
                                  p.start < midnight.getTime() &&
                                  p.end > evening.getTime() &&
                                  p.end > now,
                          )
                          .slice(0, 2)
                    : period === 'soon'
                      ? shows
                            .filter((p) => p.start > now && p.start <= now + 90 * 60_000)
                            .slice(0, 1)
                      : shows.filter((p) => p.start <= now && p.end > now).slice(-1);
            return candidates
                .filter((p) =>
                    words.every((word) =>
                        searchable(`${channel.name} ${p.title} ${p.description || ''}`).includes(
                            word,
                        ),
                    ),
                )
                .map((programme) => ({
                    channel,
                    programme,
                    key: `${channel.id}~${programme.start}`,
                }));
        })
        .sort((a, b) =>
            period !== 'now' && a.programme.start !== b.programme.start
                ? a.programme.start - b.programme.start
                : Number(favorites.includes(b.channel.id)) -
                      Number(favorites.includes(a.channel.id)) ||
                  a.channel.number - b.channel.number,
        );
}
export const time = (value: number) =>
    new Intl.DateTimeFormat('it-IT', { hour: '2-digit', minute: '2-digit' }).format(
        new Date(value),
    );
export const date = (value: number) =>
    new Intl.DateTimeFormat('it-IT', { weekday: 'long', day: 'numeric', month: 'long' }).format(
        new Date(value),
    );
export const range = (p: Programme) => `${time(p.start)} – ${time(p.end)}`;
export const remainingLabel = (p: Programme, now = Date.now()) =>
    p.start > now
        ? `Tra ${Math.max(1, Math.ceil((p.start - now) / 60_000))} min`
        : `Restano ${Math.max(0, Math.ceil((p.end - now) / 60_000))} min`;
export const providerName = (provider: string) =>
    ({
        rai: 'RAI',
        rainewsrg: 'RAI TGR',
        mediaset: 'MEDIASET',
        la7: 'LA7',
        sky: 'SKY',
        discovery: 'DISCOVERY',
        direct: 'ALTRI CANALI',
    })[provider] || provider;
