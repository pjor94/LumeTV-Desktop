import { test, expect, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { fixture, now } from './fixture';
import { defaultSettings, type PersonalSource } from '../src/personal';

const sourceId = `user_${'1'.repeat(32)}`;
const channel = (i: number, content = 'live') => ({
    id: `personal_${i.toString(16).padStart(32, '0')}`,
    source_id: sourceId,
    kind: 'tv',
    name: content === 'vod' ? `Film ${i}` : `Canale personale ${i}`,
    number: i,
    provider: 'personal',
    group: 'Notizie',
    content,
    position: i,
    color: '#7e91ac',
});
function source(): PersonalSource {
    return {
        id: sourceId,
        name: 'La mia lista',
        enabled: true,
        revision: 1,
        updated_at: now,
        checked_at: now,
        retry_at: 0,
        file: false,
        has_vod: true,
        error: '',
        report: {
            count: 500000,
            scanned: 500000,
            excluded: 0,
            duplicates: 0,
            vod: 1,
            radio: 0,
            bytes: 42000000,
            sample: ['Primo canale'],
            diagnostics: {},
            guide_urls: ['https://guide.example/tv.xml'],
            refresh_minutes: 0,
        },
        guide: {
            updated_at: now,
            loading: false,
            matched: 100,
            programmes: 3000,
            error: '',
            next_retry_at: 0,
        },
    };
}
async function personalFixture(page: Page, keySystem?: string) {
    await page.addInitScript(() =>
        Object.defineProperty(HTMLMediaElement.prototype, 'duration', { get: () => 120 }),
    );
    const base = await fixture(page);
    let settings = defaultSettings();
    settings.guide.enabled = true;
    const item = source();
    const requests: { path: string; body: Record<string, unknown> }[] = [];
    const movie = {
        id: channel(0).id,
        source: sourceId,
        title: 'Film 0',
        kind: 'movie',
        series: '',
        series_title: '',
        season: null,
        episode: null,
        year: 2020,
        duration: 120,
        description: 'Film di prova.',
        genres: [],
        groups: [],
        added: now,
        episodes: 1,
        position: 24,
        watched_duration: 120,
        completed: false,
        hidden: false,
        saved: false,
        poster: false,
        backdrop: false,
        cast: [],
        online: false,
    };
    let job = {
        id: 'test-import',
        source_id: '',
        phase: 'upload',
        bytes: 10000,
        entries: 3,
        report: {
            ...item.report,
            count: 3,
            scanned: 3,
            vod: 0,
            sample: ['Primo', 'Secondo', 'Terzo'],
        },
        error: '',
    };
    await page.route('**/api/library**', async (route) => {
        const path = new URL(route.request().url()).pathname;
        const body = route.request().headers()['content-type']?.includes('application/json')
            ? route.request().postDataJSON()
            : {};
        requests.push({ path, body });
        let result: unknown = {};
        if (path === '/api/library') result = { sources: [item] };
        else if (path === '/api/library/video/home')
            result = {
                shelves: [
                    {
                        id: 'movies',
                        title: 'Film',
                        kind: 'movie',
                        shelf: '',
                        total: 1,
                        rows: [movie],
                    },
                ],
            };
        else if (path === '/api/library/video/page') result = { rows: [movie], total: 1 };
        else if (path === '/api/library/video/detail')
            result = { item: movie, target: movie.id, target_item: movie, next: null, seasons: [] };
        else if (path === '/api/library/video/settings')
            result = { enabled: false, configured: false };
        else if (path === '/api/library/source')
            result = {
                id: item.id,
                name: item.name,
                enabled: item.enabled,
                revision: item.revision,
                url: 'https://lists.example/channels.m3u',
                settings,
            };
        else if (path === '/api/library/edit') {
            settings = body.settings;
            item.name = body.name;
            item.enabled = body.enabled;
            item.revision++;
        } else if (path === '/api/library/home') {
            const recent = base
                .preferences()
                .recent.filter((id) => id.startsWith('personal_'))
                .map((id) => channel(parseInt(id.slice(-8), 16)));
            result = { total: 500000, featured: recent[0] || channel(0), recent };
        } else if (path === '/api/library/catalog') {
            const offset = Number(body.offset || 0);
            const query = String(body.query || '');
            const favorites = body.favorites
                ? base.preferences().favorites.filter((id) => id.startsWith('personal_'))
                : null;
            const total = favorites
                ? favorites.length
                : body.content === 'vod' || query || body.group
                  ? 1
                  : 500000;
            result = {
                total,
                rows: Array.from(
                    { length: Math.max(0, Math.min(Number(body.limit || 200), total - offset)) },
                    (_, i) =>
                        channel(
                            favorites
                                ? parseInt(favorites[offset + i].slice(-8), 16)
                                : query
                                  ? 499999
                                  : body.group
                                    ? 42
                                    : offset + i,
                            String(body.content || 'live'),
                        ),
                ),
            };
        } else if (path === '/api/library/channel')
            result = channel(parseInt(String(body.id).slice(-8), 16), 'vod');
        else if (path === '/api/library/groups')
            result = { total: 2, rows: Number(body.offset) ? [] : ['Notizie', 'Cinema'] };
        else if (path === '/api/library/prepare') {
            job = { ...job, phase: body.url ? 'reading' : 'upload' };
            result = job;
        } else if (path.includes('/upload/')) {
            job = { ...job, phase: 'ready' };
            result = job;
        } else if (path === '/api/library/job') {
            if (job.phase === 'saving') job = { ...job, phase: 'done', source_id: sourceId };
            result = job;
        } else if (path === '/api/library/commit') {
            job = { ...job, phase: 'saving' };
            result = job;
        } else if (path === '/api/library/cancel') {
            job = { ...job, phase: 'cancelled' };
            result = job;
        } else if (path === '/api/library/guide/window')
            result = {
                total: 1,
                rows: [
                    {
                        title: 'Un programma da rivedere',
                        description: 'Descrizione del programma in archivio.',
                        start: now - 3600000,
                        end: now - 1800000,
                        event: String(now - 3600000),
                        archive: true,
                        ongoing: false,
                    },
                ],
            };
        else if (path === '/api/library/guide/choices')
            result = {
                total: 1,
                rows: [{ key: '0|primo', name: 'Primo canale XMLTV', xml_id: 'primo', feed: 0 }],
            };
        await route.fulfill({ json: result });
    });
    await page.route('**/api/play', async (route) => {
        const body = route.request().postDataJSON();
        if (!String(body.channel_id).startsWith('personal_')) {
            await route.fallback();
            return;
        }
        requests.push({ path: '/api/play', body });
        const content = body.mode === 'archive' ? 'archive' : body.mode === 'live' ? 'live' : 'vod';
        await route.fulfill({
            json: {
                playback_id: `fixture-${requests.length}`,
                channel: channel(0, content === 'vod' ? 'vod' : 'live'),
                stream: {
                    url: 'https://media.example/video.mpd',
                    mime_type: 'application/dash+xml',
                    mode: content,
                    title: content === 'archive' ? 'Un programma da rivedere' : 'Film 0',
                    position: body.position ?? 24,
                    duration: 120,
                    event: body.event || '',
                    seekable: true,
                    drm: keySystem
                        ? {
                              key_system: keySystem,
                              clear_keys:
                                  keySystem === 'org.w3.clearkey'
                                      ? {
                                            '00112233445566778899aabbccddeeff':
                                                '000102030405060708090a0b0c0d0e0f',
                                        }
                                      : null,
                          }
                        : null,
                },
            },
        });
    });
    await page.reload();
    await expect(page.locator('[data-route="videos"]')).toBeVisible();
    return { requests, item };
}
async function manage(page: Page) {
    await page.locator('[data-route="settings"]').click();
    await page.getByRole('button', { name: 'Gestisci liste' }).click();
}

test('an expired personal stream refreshes its remote source once and reopens the same channel', async ({
    page,
}) => {
    const { item, requests } = await personalFixture(page);
    item.checked_at = now - 3_600_000;
    let refreshes = 0;
    await page.route('**/api/library/refresh', async (route) => {
        expect(route.request().postDataJSON()).toEqual({ id: sourceId });
        refreshes++;
        await route.fulfill({ json: { id: 'expired-refresh', phase: 'reading' } });
    });
    await page.route('**/api/library/job', (route) =>
        route.fulfill({ json: { id: 'expired-refresh', phase: 'done' } }),
    );
    await page.evaluate(() => {
        const runtime = window as any;
        const original = runtime.shaka.Player.prototype.load;
        runtime.expiredLoads = 0;
        runtime.shaka.Player.prototype.load = async function (...args: unknown[]) {
            runtime.expiredLoads++;
            if (runtime.expiredLoads === 1) throw { code: 1001, data: [403] };
            return original.apply(this, args);
        };
    });
    await page.getByLabel('Lista dei canali').selectOption(sourceId);
    await page.getByRole('button', { name: 'Guarda Canale personale 0', exact: true }).click();
    await expect.poll(() => page.evaluate(() => (window as any).expiredLoads)).toBe(2);
    await expect(page.locator('#player-overlay')).toBeHidden();
    expect(refreshes).toBe(1);
    const plays = requests.filter((r) => r.path === '/api/play');
    expect(plays).toHaveLength(2);
    expect(plays.map((r) => r.body.channel_id)).toEqual([channel(0).id, channel(0).id]);
});

test('a failed refresh keeps playback recovery available and never loops source refreshes', async ({
    page,
}) => {
    const { item } = await personalFixture(page);
    item.checked_at = now - 3_600_000;
    let refreshes = 0;
    await page.route('**/api/library/refresh', async (route) => {
        refreshes++;
        await route.fulfill({ json: { id: 'failed-refresh', phase: 'reading' } });
    });
    await page.route('**/api/library/job', (route) =>
        route.fulfill({ json: { id: 'failed-refresh', phase: 'error' } }),
    );
    await page.evaluate(() => {
        (window as any).expiredLoads = 0;
        (window as any).shaka.Player.prototype.load = async function () {
            (window as any).expiredLoads++;
            throw { code: 1001, data: [403] };
        };
    });
    await page.getByLabel('Lista dei canali').selectOption(sourceId);
    await page.getByRole('button', { name: 'Guarda Canale personale 0', exact: true }).click();
    await expect(page.locator('#player-overlay')).toContainText('La sorgente non risponde.', {
        timeout: 25000,
    });
    expect(refreshes).toBe(1);
    expect(await page.evaluate(() => (window as any).expiredLoads)).toBe(4);
});

test('changing channel during a source refresh never reopens the previous channel', async ({
    page,
}) => {
    const { item, requests } = await personalFixture(page);
    item.checked_at = now - 3_600_000;
    let finished = false;
    await page.route('**/api/library/refresh', (route) =>
        route.fulfill({ json: { id: 'late-refresh', phase: 'reading' } }),
    );
    await page.route('**/api/library/job', (route) =>
        route.fulfill({ json: { id: 'late-refresh', phase: finished ? 'done' : 'reading' } }),
    );
    await page.evaluate(() => {
        const runtime = window as any;
        const original = runtime.shaka.Player.prototype.load;
        runtime.expiredLoads = 0;
        runtime.shaka.Player.prototype.load = async function (...args: unknown[]) {
            runtime.expiredLoads++;
            if (runtime.expiredLoads === 1) throw { code: 1001, data: [403] };
            return original.apply(this, args);
        };
    });
    await page.getByLabel('Lista dei canali').selectOption(sourceId);
    await page.getByRole('button', { name: 'Guarda Canale personale 0', exact: true }).click();
    await expect(page.locator('#player-overlay')).toContainText('Aggiorno la lista…');
    await page.locator('[data-route="channels"]').click();
    await page.getByRole('button', { name: 'Guarda Canale personale 1', exact: true }).click();
    await expect(page.locator('#player-overlay')).toBeHidden();
    finished = true;
    await page.waitForTimeout(600);
    expect(requests.filter((r) => r.path === '/api/play').map((r) => r.body.channel_id)).toEqual([
        channel(0).id,
        channel(1).id,
    ]);
});

test('a half-million-channel grid scrolls continuously, keeps bounded DOM and restores position', async ({
    page,
}) => {
    const { requests } = await personalFixture(page);
    await page.getByLabel('Lista dei canali').selectOption(sourceId);
    const viewport = page
        .getByRole('region', { name: 'Canali delle mie liste' })
        .locator('.continuous-viewport');
    await expect(
        page.getByRole('button', { name: 'Guarda Canale personale 0', exact: true }),
    ).toBeVisible();
    await viewport.focus();
    await page.keyboard.press('End');
    await expect(
        page.getByRole('button', { name: 'Guarda Canale personale 499999', exact: true }),
    ).toBeFocused();
    expect(await viewport.locator('.continuous-row').count()).toBeLessThan(80);
    expect(
        requests.some((r) => r.path.endsWith('/catalog') && Number(r.body.offset) >= 499800),
    ).toBeTruthy();
    await page.getByRole('button', { name: 'Guarda Canale personale 499999', exact: true }).click();
    await page.locator('[data-route="channels"]').click();
    await expect(
        page.getByRole('button', { name: 'Guarda Canale personale 499999', exact: true }),
    ).toBeVisible();
    await page.locator('[data-route="settings"]').click();
    await page.locator('[data-route="channels"]').click();
    await expect(
        page.getByRole('button', { name: 'Guarda Canale personale 499999', exact: true }),
    ).toBeVisible();
    await page.locator('#search').fill('Ultimo');
    await expect(page.getByText('1 canale', { exact: true })).toBeVisible();
    await page.getByRole('button', { name: 'Azzera filtri' }).click();
    await expect(page.locator('#search')).toHaveValue('');
});

test('personal channel cards share catalogue styling, programmes and favorite controls', async ({
    page,
}) => {
    const { requests } = await personalFixture(page);
    const builtin = page.locator('.channel-card').first();
    const appearance = await builtin.evaluate((card) => ({
        radius: getComputedStyle(card).borderRadius,
        minHeight: getComputedStyle(card.querySelector('.channel-main')!).minHeight,
        width: card.getBoundingClientRect().width,
    }));
    await page.route('**/api/library/catalog', (route) =>
        route.fulfill({
            json: {
                total: 12,
                rows: Array.from({ length: 12 }, (_, i) => ({
                    ...channel(i),
                    number: 9999,
                    ...(i === 0
                        ? {
                              now: {
                                  title: 'Il programma in onda',
                                  start: now - 1800000,
                                  end: now + 1800000,
                              },
                          }
                        : {}),
                })),
            },
        }),
    );
    await page.getByLabel('Lista dei canali').selectOption(sourceId);
    const card = page.locator('.continuous-grid .channel-card').first();
    await expect(card).toBeVisible();
    expect(await card.evaluate((el) => getComputedStyle(el).borderRadius)).toBe(appearance.radius);
    expect(
        await card.locator('.channel-main').evaluate((el) => getComputedStyle(el).minHeight),
    ).toBe(appearance.minHeight);
    expect((await card.boundingBox())!.width).toBeCloseTo(appearance.width, 0);
    await expect(card.locator('.channel-programme')).toHaveText('Il programma in onda');
    await expect(card.locator('.channel-progress i')).toHaveAttribute('style', /width: 50%/);
    await expect(card.locator('.channel-number')).toHaveText('Notizie');
    await expect(card.locator('img')).toHaveAttribute('src', `/art/channel/${channel(0).id}`);
    const favorite = card.locator('[data-favorite]');
    await favorite.click();
    await expect(favorite).toHaveAttribute('aria-pressed', 'true');
    await favorite.focus();
    await page.keyboard.press('Space');
    await expect(favorite).toHaveAttribute('aria-pressed', 'false');
    await card.locator('[data-play]').click();
    await expect.poll(() => requests.filter((r) => r.path === '/api/play').length).toBe(1);
    expect(requests.find((r) => r.path === '/api/play')?.body.channel_id).toBe(channel(0).id);
});

test('grid arrow keys follow columns and resizing preserves the current channels', async ({
    page,
}) => {
    await personalFixture(page);
    await page.getByLabel('Lista dei canali').selectOption(sourceId);
    await expect(
        page.getByRole('region', { name: 'Canale in evidenza', exact: true }),
    ).toBeVisible();
    const first = page.getByRole('button', { name: 'Guarda Canale personale 0', exact: true });
    await expect(first).toBeVisible();
    await first.focus();
    const initialScroll = await page.locator('#content-scroll').evaluate((el) => el.scrollTop);
    await page.keyboard.press('ArrowRight');
    await expect(
        page.getByRole('button', { name: 'Guarda Canale personale 1', exact: true }),
    ).toBeFocused();
    expect(await page.locator('#content-scroll').evaluate((el) => el.scrollTop)).toBe(
        initialScroll,
    );
    const columns = await page
        .locator('.continuous-canvas')
        .evaluate((el) => getComputedStyle(el).gridTemplateColumns.split(' ').length);
    await page.keyboard.press('ArrowDown');
    await expect(
        page.getByRole('button', { name: `Guarda Canale personale ${columns + 1}`, exact: true }),
    ).toBeFocused();
    await page.keyboard.press('End');
    await expect(
        page.getByRole('button', { name: 'Guarda Canale personale 499999', exact: true }),
    ).toBeFocused();
    await page.setViewportSize({ width: 820, height: 900 });
    await expect(
        page.getByRole('button', { name: 'Guarda Canale personale 499999', exact: true }),
    ).toBeVisible();
    await page.keyboard.press('Home');
    await expect(first).toBeFocused();
    await page.getByRole('button', { name: 'Tutti i gruppi', exact: true }).click();
    await page.getByRole('button', { name: 'Cinema', exact: true }).click();
    await expect(page.getByText('1 canale', { exact: true })).toBeVisible();
    await expect(
        page.getByRole('button', { name: 'Guarda Canale personale 42', exact: true }),
    ).toBeVisible();
    await page.getByRole('button', { name: 'Azzera filtri' }).click();
    await expect(first).toBeVisible();
});

test('personal grids fit the catalogue breakpoints in both themes', async ({ page }) => {
    await personalFixture(page);
    for (const width of [820, 1280, 1680])
        for (const theme of ['dark', 'light']) {
            await page.setViewportSize({ width, height: 900 });
            await page.evaluate((theme) => {
                document.documentElement.dataset.theme = theme;
            }, theme);
            await page.getByLabel('Lista dei canali').selectOption('builtin');
            const columns = await page
                .locator('.channel-grid')
                .evaluate((el) =>
                    getComputedStyle(el).gridTemplateColumns.split(' ').map(parseFloat),
                );
            await page.getByLabel('Lista dei canali').selectOption(sourceId);
            await expect(
                page.getByRole('region', { name: 'Canale in evidenza', exact: true }),
            ).toBeVisible();
            expect(
                await page
                    .locator('.hero-station .station-fallback')
                    .evaluate(
                        (el) =>
                            el.scrollWidth <= el.clientWidth + 1 &&
                            el.scrollHeight <= el.clientHeight + 1,
                    ),
            ).toBeTruthy();
            const grid = page.locator('.continuous-canvas.channel-grid');
            await expect(grid.locator('.channel-card').first()).toBeVisible();
            await expect
                .poll(() =>
                    grid.evaluate(
                        (el) => getComputedStyle(el).gridTemplateColumns.split(' ').length,
                    ),
                )
                .toBe(columns.length);
            const tracks = await grid.evaluate((el) =>
                getComputedStyle(el).gridTemplateColumns.split(' ').map(parseFloat),
            );
            tracks.forEach((width, index) => expect(width).toBeCloseTo(columns[index], 0));
            expect(
                await page
                    .locator('.continuous-viewport')
                    .evaluate((el) => getComputedStyle(el).overflowY),
            ).toBe('visible');
            expect(await grid.locator('.channel-card').count()).toBeLessThan(80);
            expect(
                await page
                    .locator('#content-scroll')
                    .evaluate((el) => el.scrollWidth <= el.clientWidth + 1),
            ).toBeTruthy();
            await page.mouse.move(20, 20);
            await page.screenshot({
                path: `../../reports/personal-catalogue-${theme}-${width}.png`,
                animations: 'disabled',
            });
        }
});

test('personal home displays featured programmes, recent channels and opens the matching guide', async ({
    page,
}) => {
    const { requests } = await personalFixture(page);
    const featured = {
        ...channel(3),
        now: {
            title: 'Un programma dalla mia lista',
            description: 'La descrizione del programma.',
            start: now - 1800000,
            end: now + 1800000,
        },
    };
    await page.route('**/api/library/home', (route) =>
        route.fulfill({ json: { total: 500000, featured, recent: [featured, channel(1)] } }),
    );
    await page.getByLabel('Lista dei canali').selectOption(sourceId);
    const hero = page.getByRole('region', { name: 'Canale in evidenza', exact: true });
    await expect(hero).toContainText(featured.now.title);
    await expect(hero).toContainText(featured.now.description);
    await expect(
        hero.getByRole('button', { name: 'Guarda ora Canale personale 3', exact: true }),
    ).toBeVisible();
    const recent = page.getByRole('region', { name: 'Visti di recente', exact: true });
    await expect(recent.locator('[data-play]')).toHaveCount(2);
    await expect(recent.locator('[data-play]').first()).toHaveAttribute('data-play', featured.id);
    expect(
        await recent
            .locator('.station-fallback')
            .first()
            .evaluate((el) => el.scrollHeight <= el.clientHeight + 1),
    ).toBeTruthy();
    expect(requests.filter((r) => r.path === '/api/play')).toHaveLength(0);
    await page.mouse.move(20, 20);
    await page.screenshot({
        path: '../../reports/personal-home-programmes-and-recent.png',
        animations: 'disabled',
    });
    await page
        .locator('.section-heading')
        .getByRole('button', { name: 'Apri la guida', exact: true })
        .click();
    await expect(
        page.getByRole('heading', { name: 'La guida di La mia lista.', exact: true }),
    ).toBeVisible();
    expect(
        await page.evaluate(() =>
            JSON.parse(sessionStorage.getItem('lume-guide-source') || 'null'),
        ),
    ).toBe(sourceId);
    expect(
        await page.evaluate(
            (source) =>
                JSON.parse(sessionStorage.getItem(`lume-guide-channel-${source}`) || 'null')?.id,
            sourceId,
        ),
    ).toBe(featured.id);
});

test('public Lume and EPGShare metadata refresh the personal home, guide and cached logos', async ({
    page,
}) => {
    await page.clock.install({ time: now });
    const { item } = await personalFixture(page);
    item.report.guide_urls = [];
    item.guide = { ...item.guide, origin: 'lume', channel_count: 514, matched: 0 };
    item.guide.supplement = {
        name: 'EPGShare IT1',
        channel_count: 195,
        programmes: 10000,
        updated_at: now,
        loading: false,
        error: '',
        next_retry_at: 0,
    };
    let revision = 1;
    const supplemented = () => ({
        ...channel(0),
        guide_origin: 'lume',
        artwork_revision: `lume:${revision}`,
        now: {
            title: `Programma dal feed completo ${revision}`,
            description: 'Programmazione di un canale esterno al Catalogo Lume.',
            start: now - 1800000,
            end: now + 1800000,
        },
    });
    await page.route('**/api/library', (route) =>
        route.fulfill({ json: { sources: [item], metadata_revision: `public:${revision}` } }),
    );
    await page.route('**/api/library/home', (route) =>
        route.fulfill({ json: { total: 1, featured: supplemented(), recent: [] } }),
    );
    await page.route('**/api/library/catalog', (route) =>
        route.fulfill({ json: { total: 1, rows: [supplemented()] } }),
    );
    await page.route('**/api/library/guide/window', (route) =>
        route.fulfill({
            json: {
                total: 1,
                rows: [
                    {
                        ...supplemented().now,
                        event: 'lume:fixture',
                        archive: false,
                        ongoing: false,
                    },
                ],
            },
        }),
    );
    const png = Buffer.from(
        readFileSync(
            new URL('../../../tests/fixtures/guide-logo.png.base64', import.meta.url),
            'utf8',
        ),
        'base64',
    );
    await page.route(`**/art/channel/${channel(0).id}*`, (route) =>
        route.fulfill({ contentType: 'image/png', body: png }),
    );
    await page.reload();
    await page.getByLabel('Lista dei canali').selectOption(sourceId);
    const hero = page.getByRole('region', { name: 'Canale in evidenza', exact: true });
    await expect(hero).toContainText('Programma dal feed completo 1');
    const card = page.locator('.continuous-grid .channel-card').first();
    await expect(card.locator('.channel-programme')).toHaveText('Programma dal feed completo 1');
    await expect(card.locator('img')).toHaveAttribute('src', /\?v=lume%3A1$/);
    await expect
        .poll(() => card.locator('img').evaluate((el) => (el as HTMLImageElement).naturalWidth))
        .toBeGreaterThan(0);

    // The personal source and private guide timestamps stay unchanged.
    revision = 2;
    await page.clock.fastForward(31_000);
    await expect(card.locator('.channel-programme')).toHaveText('Programma dal feed completo 2');
    await expect(card.locator('img')).toHaveAttribute('src', /\?v=lume%3A2$/);
    await page
        .locator('.section-heading')
        .getByRole('button', { name: 'Apri la guida', exact: true })
        .click();
    await expect(page.getByText(/Guida integrata da Lume/)).toBeVisible();
    await expect(page.getByText(/EPGShare IT1 completa la programmazione/)).toBeVisible();
    await expect(page.getByLabel('Cerca nei programmi')).toBeVisible();
    await expect(page.locator('.personal-programme strong')).toHaveText(
        'Programma dal feed completo 2',
    );
    await expect(page.getByRole('button', { name: 'Rivedi dall’archivio' })).toHaveCount(0);
    await page.getByRole('button', { name: 'Dettagli', exact: true }).click();
    await expect(
        page.getByRole('heading', { name: 'Programma dal feed completo 2' }),
    ).toBeFocused();
    await page.screenshot({
        path: '../../reports/lume-guide-fallback.png',
        animations: 'disabled',
    });
    await manage(page);
    await page.getByRole('button', { name: 'La mia lista', exact: false }).click();
    await page.getByRole('button', { name: 'Stato e ultima importazione', exact: true }).click();
    await expect(page.getByText('Guida Lume · 514 canali nel feed completo')).toBeVisible();
    await expect(page.getByText('Integrazione EPGShare IT1', { exact: true })).toBeVisible();
    await expect(page.getByText('195 canali nella guida integrativa')).toBeVisible();
    await page.getByRole('button', { name: 'Guida e archivio', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Aggiorna guida', exact: true })).toBeEnabled();
    await expect(page.getByRole('button', { name: 'Associa canali', exact: true })).toHaveCount(0);
    await expect(
        page.getByRole('button', { name: 'Svuota guida scaricata', exact: true }),
    ).toHaveCount(0);
});

test('a delayed home reply cannot replace the next selected source', async ({ page }) => {
    const { item } = await personalFixture(page);
    const secondId = `user_${'2'.repeat(32)}`;
    await page.route('**/api/library', (route) =>
        route.fulfill({
            json: { sources: [item, { ...item, id: secondId, name: 'Seconda lista' }] },
        }),
    );
    let started = false;
    await page.route('**/api/library/home', async (route) => {
        const selected = route.request().postDataJSON().source;
        if (selected === sourceId) {
            started = true;
            await new Promise((resolve) => setTimeout(resolve, 300));
        }
        await route
            .fulfill({
                json: {
                    total: 1,
                    featured: {
                        ...channel(0),
                        source_id: selected,
                        name:
                            selected === sourceId ? 'Vecchia lista' : 'Canale della seconda lista',
                    },
                    recent: [],
                },
            })
            .catch(() => {});
    });
    await page.reload();
    await page.getByLabel('Lista dei canali').selectOption(sourceId);
    await expect.poll(() => started).toBeTruthy();
    await page.getByLabel('Lista dei canali').selectOption(secondId);
    const hero = page.getByRole('region', { name: 'Canale in evidenza', exact: true });
    await expect(hero).toContainText('Canale della seconda lista');
    await page.waitForTimeout(350);
    await expect(hero).not.toContainText('Vecchia lista');
    await page.locator('#search').fill('Ultimo');
    await expect(hero).toHaveCount(0);
});

test('file import verifies a preview before saving and keeps form fields across sections', async ({
    page,
}) => {
    const { requests } = await personalFixture(page);
    await manage(page);
    await page.getByRole('button', { name: 'Aggiungi lista' }).click();
    await page.getByLabel('Nome della lista', { exact: true }).fill('Lista dal file');
    await page.getByRole('button', { name: 'Da file', exact: true }).click();
    await page.getByLabel('File della lista').setInputFiles({
        name: 'prova.m3u',
        mimeType: 'text/plain',
        buffer: Buffer.from('#EXTM3U\n#EXTINF:-1,Primo\nhttps://media.example/first.m3u8\n'),
    });
    await page.getByRole('button', { name: 'Accesso ai server', exact: true }).click();
    await page.getByLabel('Header HTTP', { exact: true }).fill('X-Example: fixture');
    await page.getByRole('button', { name: 'Sorgente', exact: true }).click();
    await expect(page.getByLabel('Nome della lista', { exact: true })).toHaveValue(
        'Lista dal file',
    );
    await page.getByRole('button', { name: 'Verifica lista', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Anteprima pronta' })).toBeVisible();
    expect(requests.some((r) => r.path.endsWith('/commit'))).toBeFalsy();
    await page.getByRole('button', { name: 'Importa lista', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Lista salvata', exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Apri il catalogo' })).toBeVisible();
});

test('unsaved changes and deletion use in-page confirmations', async ({ page }) => {
    await personalFixture(page);
    await manage(page);
    await page.getByRole('button', { name: 'La mia lista', exact: false }).click();
    await page.getByLabel('Nome della lista', { exact: true }).fill('Nome modificato');
    await page.getByRole('button', { name: 'Le mie liste', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Continua a modificare' })).toBeVisible();
    await page.getByRole('button', { name: 'Continua a modificare' }).click();
    await expect(page.getByLabel('Nome della lista', { exact: true })).toHaveValue(
        'Nome modificato',
    );
    await page.getByRole('button', { name: 'Elimina lista', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Annulla', exact: true })).toBeFocused();
    await page.getByRole('button', { name: 'Annulla', exact: true }).click();
    await expect(page.getByLabel('Nome della lista', { exact: true })).toHaveValue(
        'Nome modificato',
    );
});

test('VOD finishes without live recovery and can restart, seek and keep one player', async ({
    page,
}) => {
    const { requests } = await personalFixture(page);
    await page.locator('[data-route="videos"]').click();
    await page.getByRole('button', { name: 'Dettagli di Film 0', exact: true }).click();
    await page.getByRole('button', { name: 'Riprendi da 0:24', exact: true }).click();
    await expect(page.locator('#play-pause')).toHaveAttribute('aria-label', 'Pausa');
    await expect(page.getByLabel('Posizione del video')).toBeVisible();
    await page.locator('#play-pause').click();
    await page.locator('#play-pause').click();
    expect(requests.filter((r) => r.path === '/api/play')).toHaveLength(1);
    await page.locator('#video').evaluate((video: HTMLVideoElement) => {
        video.currentTime = 120;
        video.dispatchEvent(new Event('ended'));
    });
    await expect(page.locator('#play-pause')).toHaveAttribute(
        'aria-label',
        'Riproduci dall’inizio',
    );
    await page.waitForTimeout(2300);
    expect(requests.filter((r) => r.path === '/api/play')).toHaveLength(1);
    expect(
        requests.some((r) => r.path.endsWith('/watch') && r.body.completed === true),
    ).toBeTruthy();
    await page.locator('#play-pause').click();
    await expect.poll(() => requests.filter((r) => r.path === '/api/play').length).toBe(2);
    expect(requests.filter((r) => r.path === '/api/play')[1].body.position).toBe(0);
    expect(await page.evaluate(() => (window as any).testPlayer.attachments)).toBe(1);
});

test('personal guide offers archive playback and an explicit return to live', async ({ page }) => {
    const { requests } = await personalFixture(page);
    await page.locator('.main-navigation [data-route="guide"]').click();
    await page.getByLabel('Guida della lista').selectOption(sourceId);
    await page.getByRole('button', { name: 'Dettagli', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Un programma da rivedere' })).toBeFocused();
    await page.getByRole('button', { name: 'Chiudi dettagli' }).click();
    await expect(page.getByRole('button', { name: 'Dettagli', exact: true })).toBeFocused();
    await page.getByRole('button', { name: 'Rivedi dall’archivio', exact: true }).click();
    await expect(page.locator('#watch-details h1')).toHaveText('Un programma da rivedere');
    await page
        .locator('.video-seek-actions')
        .getByRole('button', { name: 'Torna alla diretta' })
        .click();
    await expect
        .poll(() => requests.filter((r) => r.path === '/api/play').at(-1)?.body.mode)
        .toBe('live');
});

test('ClearKey selects its own license system and changing to clear content resets DRM', async ({
    page,
}) => {
    await personalFixture(page, 'org.w3.clearkey');
    await page.locator('[data-route="videos"]').click();
    await page.getByRole('button', { name: 'Dettagli di Film 0', exact: true }).click();
    await page.getByRole('button', { name: 'Riprendi da 0:24', exact: true }).click();
    await expect
        .poll(() =>
            page.evaluate(
                () =>
                    (window as any).testPlayer.configurations.find(
                        (c: any) => c.drm?.servers?.['org.w3.clearkey'],
                    )?.drm.servers,
            ),
        )
        .toEqual({ 'org.w3.clearkey': 'lume-license:license' });
    expect(
        await page.evaluate(
            () =>
                (window as any).testPlayer.configurations.find(
                    (c: any) => c.drm?.clearKeys?.['00112233445566778899aabbccddeeff'],
                )?.drm.clearKeys,
        ),
    ).toEqual({ '00112233445566778899aabbccddeeff': '000102030405060708090a0b0c0d0e0f' });
    await page.locator('[data-route="channels"]').click();
    await page.locator('[data-play="rai1"]').first().click();
    await expect
        .poll(() =>
            page.evaluate(
                () =>
                    (window as any).testPlayer.configurations.filter((c: any) => c.drm).at(-1)?.drm
                        .servers,
            ),
        )
        .toEqual({});
    expect(
        await page.evaluate(
            () =>
                (window as any).testPlayer.configurations.filter((c: any) => c.drm).at(-1)?.drm
                    .clearKeys,
        ),
    ).toEqual({});
    expect(await page.evaluate(() => (window as any).testPlayer.attachments)).toBe(1);
});

test('new screens fit both themes at 820 and 1280 pixels', async ({ page }) => {
    await personalFixture(page);
    await manage(page);
    await page.getByRole('button', { name: 'La mia lista', exact: false }).click();
    for (const width of [820, 1280])
        for (const theme of ['dark', 'light']) {
            await page.setViewportSize({ width, height: 900 });
            await page.evaluate((theme) => (document.documentElement.dataset.theme = theme), theme);
            await page.getByRole('button', { name: 'Accesso ai server', exact: true }).click();
            await expect(
                page.getByRole('heading', { name: 'Accesso ai server', exact: true }),
            ).toBeVisible();
            await page.mouse.move(20, 20);
            await page.screenshot({
                path: `../../reports/personal-access-${theme}-${width}.png`,
                animations: 'disabled',
            });
            expect(
                await page
                    .locator('.library-editor')
                    .evaluate((el) => el.scrollWidth <= el.clientWidth + 1),
            ).toBeTruthy();
            await page.getByRole('button', { name: 'Guida e archivio', exact: true }).click();
            await expect(
                page.getByRole('heading', { name: 'Guida e archivio', exact: true }),
            ).toBeVisible();
            await page.mouse.move(20, 20);
            await page.screenshot({
                path: `../../reports/personal-guide-settings-${theme}-${width}.png`,
                animations: 'disabled',
            });
        }
});
