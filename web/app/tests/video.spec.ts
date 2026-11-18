import { test, expect, type Page } from '@playwright/test';
import { mkdir } from 'node:fs/promises';
import { fixture, now } from './fixture';
import type { VideoItem, VideoQuery } from '../src/video';

const source = `user_${'8'.repeat(32)}`;
const other = `user_${'9'.repeat(32)}`;
const id = (n: number) => `personal_${n.toString(16).padStart(32, '0')}`;
function item(n: number, overrides: Partial<VideoItem> = {}): VideoItem {
    return {
        id: id(n),
        source,
        title: `Film di prova ${n}`,
        kind: 'movie',
        series: '',
        series_title: '',
        season: null,
        episode: null,
        year: 2020,
        duration: 7200,
        description:
            'Una trama di prova per verificare la libreria, i dettagli del titolo e la continuità della visione.',
        genres: ['Avventura', 'Dramma'],
        groups: ['Cinema'],
        added: now - n * 1000,
        episodes: 1,
        position: 0,
        watched_duration: 0,
        completed: false,
        hidden: false,
        saved: false,
        poster: true,
        backdrop: true,
        cast: [],
        directors: [],
        overview_language: '',
        online: false,
        ...overrides,
    };
}
async function videosFixture(page: Page) {
    const base = await fixture(page);
    const rows = [
        item(100, { title: 'Il viaggio', position: 240, watched_duration: 7200 }),
        item(101, { title: 'Una sera al mare', poster: false, backdrop: false }),
        ...Array.from({ length: 5 }, (_, i) =>
            item(200 + i, {
                title: `La nostra serie S0${i < 3 ? 1 : 2}E0${i < 3 ? i + 1 : i - 2}`,
                kind: 'episode',
                series: 'series-key',
                series_title: 'La nostra serie',
                season: i < 3 ? 1 : 2,
                episode: i < 3 ? i + 1 : i - 2,
                groups: ['Serie'],
                duration: 1800,
                position: i === 1 ? 24 : 0,
                watched_duration: i === 1 ? 1800 : 0,
            }),
        ),
        item(300, { source: other, title: 'Un’altra lista' }),
    ];
    let online = false;
    let configured = false;
    const requests: { path: string; body: any }[] = [];
    const query = (q: VideoQuery) => {
        let filtered = rows.filter(
            (i) =>
                (!q.source || i.source === q.source) &&
                (!q.kind || i.kind === q.kind) &&
                (!q.query ||
                    `${i.title} ${i.series_title}`.toLowerCase().includes(q.query.toLowerCase())) &&
                (!q.group || i.groups.includes(q.group)) &&
                (!q.series || i.series === q.series) &&
                (q.season == null || i.season === q.season) &&
                (q.shelf !== 'continue' || (i.position >= 5 && !i.completed && !i.hidden)) &&
                (q.shelf !== 'saved' || i.saved),
        );
        if (q.order === 'title')
            filtered.sort((a, b) =>
                (a.series_title || a.title).localeCompare(b.series_title || b.title),
            );
        if (q.series)
            filtered.sort(
                (a, b) => (a.season || 0) - (b.season || 0) || (a.episode || 0) - (b.episode || 0),
            );
        if (!q.episodes && q.shelf !== 'continue')
            filtered = filtered
                .filter(
                    (i, index, all) =>
                        !i.series ||
                        all.findIndex((a) => a.source === i.source && a.series === i.series) ===
                            index,
                )
                .map((i) => ({
                    ...i,
                    episodes: i.series
                        ? rows.filter((a) => a.source === i.source && a.series === i.series).length
                        : 1,
                }));
        return {
            total: filtered.length,
            rows: filtered.slice(q.offset || 0, (q.offset || 0) + (q.limit || 100)),
        };
    };
    const detail = (value: VideoItem) => {
        const episodes = rows.filter(
            (i) => i.series && i.series === value.series && i.source === value.source,
        );
        const target =
            episodes.find((i) => i.position >= 5 && !i.completed && !i.hidden) ||
            episodes.find((i) => !i.completed) ||
            value;
        const next = episodes[episodes.findIndex((i) => i.id === value.id) + 1]?.id || null;
        return {
            item: { ...value, episodes: episodes.length || 1 },
            target: target.id,
            target_item: target,
            next: value.series ? next : null,
            seasons: [1, 2]
                .filter((season) => episodes.some((i) => i.season === season))
                .map((season) => ({
                    season,
                    episodes: episodes.filter((i) => i.season === season).length,
                })),
        };
    };
    await page.route('**/art/video/**', async (route) => {
        await route.fulfill({
            contentType: 'image/svg+xml',
            body: '<svg xmlns="http://www.w3.org/2000/svg" width="360" height="540"><rect width="360" height="540" fill="#26394f"/><path d="M40 360H320M40 375H230" stroke="#99caff" stroke-width="3"/><text x="40" y="300" font-size="32" fill="#f5f6f8" font-family="sans-serif">Lume</text><text x="40" y="334" font-size="16" fill="#c1cbd7" font-family="sans-serif">Locandina di prova</text></svg>',
        });
    });
    await page.route('**/api/library**', async (route) => {
        const path = new URL(route.request().url()).pathname;
        const body = route.request().method() === 'POST' ? route.request().postDataJSON() : {};
        requests.push({ path, body });
        let result: unknown = {};
        if (path === '/api/library')
            result = {
                sources: [source, other].map((s, i) => ({
                    id: s,
                    name: i ? 'Seconda lista' : 'La mia videoteca',
                    enabled: true,
                    revision: 1,
                    has_vod: true,
                    updated_at: now,
                    checked_at: now,
                    retry_at: 0,
                    file: true,
                    error: '',
                    guide: {
                        updated_at: 0,
                        loading: false,
                        matched: 0,
                        programmes: 0,
                        error: '',
                        next_retry_at: 0,
                    },
                    report: {
                        count: 8,
                        vod: 8,
                        radio: 0,
                        scanned: 8,
                        excluded: 0,
                        duplicates: 0,
                        bytes: 1000,
                        sample: [],
                        diagnostics: {},
                        guide_urls: [],
                        refresh_minutes: 0,
                    },
                })),
            };
        else if (path === '/api/library/video/home')
            result = {
                shelves: [
                    ['continue', 'Continua a guardare', '', 'continue'],
                    ['saved', 'La mia lista', '', 'saved'],
                    ['movies', 'Film', 'movie', ''],
                    ['series', 'Serie', 'episode', ''],
                    ['added', 'Aggiunti di recente', '', 'added'],
                ]
                    .map(([s, title, kind, shelf]) => ({
                        id: s,
                        title,
                        kind,
                        shelf,
                        ...query({ source: body.source, kind, shelf }),
                    }))
                    .filter((s) => s.total),
            };
        else if (path === '/api/library/video/page') {
            if (body.query === 'grande')
                result = {
                    total: 500000,
                    rows: Array.from(
                        { length: Math.min(body.limit || 100, 500000 - body.offset) },
                        (_, n) =>
                            item(1000 + n + body.offset, {
                                title: `Video ${n + body.offset}`,
                                poster: false,
                                backdrop: false,
                            }),
                    ),
                };
            else result = query(body);
        } else if (path === '/api/library/video/detail')
            result = detail(rows.find((i) => i.id === body.id) || item(0));
        else if (path === '/api/library/video/action') {
            const value = rows.find((i) => i.id === body.id)!;
            if (body.action === 'saved')
                rows.filter(
                    (i) =>
                        i.id === value.id ||
                        (value.series && i.series === value.series && i.source === value.source),
                ).forEach((i) => {
                    i.saved = body.enabled;
                });
            else if (body.action === 'completed') {
                value.completed = body.enabled;
                if (!body.enabled) value.position = 0;
            } else if (body.action === 'hidden') value.hidden = body.enabled;
            result = { saved: true, preferences: base.preferences() };
        } else if (path === '/api/library/video/settings') {
            if (body.token) configured = true;
            if (body.enabled !== undefined) online = body.enabled;
            result = { configured, enabled: online };
        } else if (path === '/api/library/video/metadata')
            result = {
                status: 'ambiguous',
                candidates: [
                    {
                        id: 42,
                        kind: 'movie',
                        title: 'Il viaggio corretto',
                        original_title: '',
                        year: 2021,
                    },
                ],
            };
        else if (path === '/api/library/video/choose') {
            const value = rows.find((i) => i.id === body.id)!;
            value.online = body.candidate != null;
            value.description = body.candidate
                ? 'Trama dal titolo associato manualmente.'
                : 'Trama originale.';
            value.cast = body.candidate ? ['Attrice di prova', 'Attore di prova'] : [];
            result = { status: body.candidate ? 'ready' : 'excluded', candidates: [] };
        } else if (path === '/api/library/groups') result = { rows: ['Cinema', 'Serie'], total: 2 };
        else if (path === '/api/library/channel') {
            const value = rows.find((i) => i.id === body.id)!;
            result = {
                id: value.id,
                source_id: value.source,
                content: 'vod',
                kind: 'tv',
                name: value.title,
                number: 9999,
                provider: 'personal',
                group: value.groups.join(' · '),
                color: '#7e91ac',
            };
        }
        await route.fulfill({ json: result });
    });
    await page.route('**/api/play', async (route) => {
        const body = route.request().postDataJSON();
        requests.push({ path: '/api/play', body });
        const value = rows.find((i) => i.id === body.channel_id)!;
        await route.fulfill({
            json: {
                playback_id: `video-${value.id}`,
                channel: {
                    id: value.id,
                    name: value.title,
                    kind: 'tv',
                    source_id: value.source,
                    content: 'vod',
                    provider: 'personal',
                    number: 9999,
                    group: 'Video',
                    color: '#7e91ac',
                },
                stream: {
                    url: 'https://media.example/test.mpd',
                    mime_type: 'application/dash+xml',
                    mode: 'vod',
                    title: value.title,
                    position: body.position ?? value.position,
                    duration: value.duration,
                    seekable: true,
                    watch_revision: 1,
                },
            },
        });
    });
    await page.goto('/#fixture_session_token_123456789');
    await expect(page.locator('[data-route="videos"]')).toBeVisible();
    await page.locator('[data-route="videos"]').click();
    await expect(page.getByRole('heading', { name: 'I tuoi film. Le tue serie.' })).toBeVisible();
    return { ...base, requests, rows };
}

test('video shelves, series seasons, episode details and next episode preserve a single player', async ({
    page,
}) => {
    const { requests } = await videosFixture(page);
    await expect(
        page.getByRole('region', { name: 'Serie', exact: true }).locator('.video-card'),
    ).toHaveCount(1);
    await page
        .getByRole('region', { name: 'Serie', exact: true })
        .getByRole('button', { name: 'Dettagli di La nostra serie', exact: true })
        .click();
    await expect(page.locator('.video-detail-poster .video-art')).toHaveJSProperty(
        'naturalWidth',
        360,
    );
    await expect(page.getByRole('heading', { name: 'La nostra serie', exact: true })).toBeFocused();
    await expect(page.getByRole('button', { name: 'Stagione 1 3', exact: true })).toHaveAttribute(
        'aria-pressed',
        'true',
    );
    await expect(page.locator('.video-episode')).toHaveCount(3);
    await page.getByRole('button', { name: 'Stagione 2 2', exact: true }).click();
    await expect(page.locator('.video-episode')).toHaveCount(2);
    await page
        .getByRole('button', { name: 'Dettagli di La nostra serie S02E01', exact: true })
        .click();
    await page.getByRole('button', { name: 'Segna come visto', exact: true }).click();
    await expect(
        page.getByRole('button', { name: 'Segna come non visto', exact: true }),
    ).toBeVisible();
    await page.getByRole('button', { name: 'Torna ai video', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'La nostra serie', exact: true })).toBeVisible();
    await page.getByRole('button', { name: 'Riprendi da 0:24', exact: true }).click();
    await expect(page.locator('#play-pause')).toHaveAttribute('aria-label', 'Pausa');
    await page
        .locator('#video')
        .evaluate((video: HTMLVideoElement) => video.dispatchEvent(new Event('ended')));
    await page
        .locator('.video-seek-actions')
        .getByRole('button', { name: 'Prossimo episodio', exact: true })
        .click();
    await expect
        .poll(() => requests.filter((r) => r.path === '/api/play').at(-1)?.body.channel_id)
        .toBe(id(202));
    expect(await page.evaluate(() => (window as any).testPlayer.attachments)).toBe(1);
});

test('save, continue visibility and watched decisions update the video library', async ({
    page,
}) => {
    const { requests } = await videosFixture(page);
    await page
        .getByRole('region', { name: 'Film', exact: true })
        .getByRole('button', { name: 'Dettagli di Il viaggio', exact: true })
        .click();
    await page.getByRole('button', { name: 'La mia lista', exact: true }).click();
    await expect(
        page.getByRole('button', { name: 'Nella mia lista', exact: true }),
    ).toHaveAttribute('aria-pressed', 'true');
    await page.getByRole('button', { name: 'Rimuovi da Continua a guardare', exact: true }).click();
    await expect(
        page.getByRole('button', { name: 'Mostra in Continua a guardare', exact: true }),
    ).toBeVisible();
    await page.getByRole('button', { name: 'Torna ai video', exact: true }).click();
    await expect(
        page
            .getByRole('region', { name: 'La mia lista', exact: true })
            .getByRole('button', { name: 'Dettagli di Il viaggio', exact: true }),
    ).toBeVisible();
    await expect(
        page
            .getByRole('region', { name: 'Continua a guardare', exact: true })
            .getByRole('button', { name: 'Dettagli di Il viaggio', exact: true }),
    ).toHaveCount(0);
    await page
        .getByRole('region', { name: 'La mia lista', exact: true })
        .getByRole('button', { name: 'Dettagli di Il viaggio', exact: true })
        .click();
    await page.getByRole('button', { name: 'Segna come visto', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Riguarda', exact: true })).toBeVisible();
    expect(
        requests.some(
            (r) => r.path.endsWith('/action') && r.body.action === 'completed' && r.body.enabled,
        ),
    ).toBeTruthy();
});

test('large video catalog is virtual, searchable and keyboard navigable', async ({ page }) => {
    await videosFixture(page);
    await page.locator('#search').fill('grande');
    await expect(page.locator('.library-result-count')).toContainText('500.000 titoli');
    await expect(
        page.getByRole('button', { name: 'Dettagli di Video 0', exact: true }),
    ).toBeVisible();
    await page.getByRole('button', { name: 'Dettagli di Video 0', exact: true }).focus();
    await page.keyboard.press('End');
    await expect(
        page.getByRole('button', { name: 'Dettagli di Video 499999', exact: true }),
    ).toBeFocused();
    expect(await page.locator('.video-card').count()).toBeLessThan(80);
    await page.getByLabel('Ordine dei video').selectOption('title');
    await page.locator('#search').fill('viaggio');
    await expect(page.locator('.library-result-count')).toContainText('1 titolo');
});

test('online metadata requires configuration and opt in and supports manual title selection', async ({
    page,
}) => {
    const { requests } = await videosFixture(page);
    await page.getByRole('button', { name: 'Informazioni online', exact: true }).click();
    await expect(page.getByLabel('Usa le informazioni di TMDB')).toBeDisabled();
    await page.getByLabel('Read Access Token TMDB', { exact: true }).fill('fixture-token');
    await page.getByRole('button', { name: 'Salva token', exact: true }).click();
    await page.getByLabel('Usa le informazioni di TMDB').check();
    await page
        .getByRole('region', { name: 'Informazioni online', exact: true })
        .getByRole('button', { name: 'Chiudi', exact: true })
        .click();
    await page
        .getByRole('region', { name: 'Film', exact: true })
        .getByRole('button', { name: 'Dettagli di Il viaggio', exact: true })
        .click();
    await expect(
        page.getByRole('button', { name: 'Il viaggio corretto · 2021', exact: true }),
    ).toBeVisible();
    await page.getByRole('button', { name: 'Il viaggio corretto · 2021', exact: true }).click();
    await expect(
        page.getByText('Trama dal titolo associato manualmente.', { exact: true }),
    ).toBeVisible();
    await expect(
        page.getByText('Attrice di prova, Attore di prova', { exact: false }),
    ).toBeVisible();
    expect(
        requests
            .filter((r) => r.path.endsWith('/metadata'))
            .every((r) => Object.keys(r.body).every((k) => ['id', 'query'].includes(k))),
    ).toBeTruthy();
});

for (const failure of ['NotAllowedError', 'NotSupportedError'])
    test(`VOD reports ${failure} with the correct action`, async ({ page }) => {
        await videosFixture(page);
        await page.evaluate((failure) => {
            HTMLMediaElement.prototype.play = async function () {
                throw new DOMException('Fixture media failure', failure);
            };
        }, failure);
        await page
            .getByRole('region', { name: 'Film', exact: true })
            .getByRole('button', { name: 'Dettagli di Il viaggio', exact: true })
            .click();
        await page.getByRole('button', { name: 'Riprendi da 4:00', exact: true }).click();
        await expect(page.locator('.live-badge')).toHaveText('VIDEO');
        if (failure === 'NotAllowedError') {
            await expect(
                page.getByRole('button', { name: 'Avvia il video', exact: true }),
            ).toBeVisible();
            await expect(
                page.getByRole('heading', { name: 'Il tuo video è pronto.', exact: true }),
            ).toBeVisible();
        } else {
            await expect(
                page.getByRole('heading', { name: 'Non riesco ad avviare il video.', exact: true }),
            ).toBeVisible();
            await expect(page.locator('#player-retry')).toBeVisible();
            await expect(
                page.getByRole('button', { name: 'Avvia il video', exact: true }),
            ).toBeHidden();
        }
    });

test('seeking after the end resumes from the chosen point without restarting the session', async ({
    page,
}) => {
    const { requests } = await videosFixture(page);
    await page
        .getByRole('region', { name: 'Film', exact: true })
        .getByRole('button', { name: 'Dettagli di Il viaggio', exact: true })
        .click();
    await page.getByRole('button', { name: 'Riprendi da 4:00', exact: true }).click();
    await expect(page.locator('#play-pause')).toHaveAttribute('aria-label', 'Pausa');
    await page
        .locator('#video')
        .evaluate((video: HTMLVideoElement) => video.dispatchEvent(new Event('ended')));
    await expect(page.locator('#play-pause')).toHaveAttribute(
        'aria-label',
        'Riproduci dall’inizio',
    );
    await page.getByLabel('Posizione del video').focus();
    await page.keyboard.press('Home');
    await page.keyboard.press('ArrowRight');
    await expect(page.locator('#play-pause')).toHaveAttribute('aria-label', 'Riprendi');
    await page.locator('#play-pause').click();
    await expect(page.locator('#play-pause')).toHaveAttribute('aria-label', 'Pausa');
    expect(requests.filter((r) => r.path === '/api/play')).toHaveLength(1);
});

test('video layouts remain usable in both themes at desktop sizes', async ({ page }) => {
    await videosFixture(page);
    await mkdir('../../.cache/vod-visual', { recursive: true });
    for (const width of [820, 1280, 1680])
        for (const theme of ['dark', 'light']) {
            await page.setViewportSize({ width, height: 960 });
            await page.evaluate((theme) => {
                document.documentElement.dataset.theme = theme;
            }, theme);
            await expect(page.locator('.video-card').first()).toBeVisible();
            await page.waitForTimeout(200);
            await page.screenshot({ path: `../../.cache/vod-visual/home-${width}-${theme}.png` });
            expect(
                await page.evaluate(() => document.documentElement.scrollWidth > innerWidth),
            ).toBe(false);
        }
    await page
        .getByRole('region', { name: 'Serie', exact: true })
        .getByRole('button', { name: 'Dettagli di La nostra serie', exact: true })
        .click();
    for (const width of [820, 1280]) {
        await page.setViewportSize({ width, height: 960 });
        await expect(page.locator('.video-detail h1')).toBeVisible();
        await expect(
            page.getByRole('button', { name: 'Stagione 1 3', exact: true }),
        ).toHaveAttribute('aria-pressed', 'true');
        await page.waitForTimeout(200);
        await page.screenshot({ path: `../../.cache/vod-visual/detail-${width}.png` });
        expect(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)).toBe(
            false,
        );
    }
});
