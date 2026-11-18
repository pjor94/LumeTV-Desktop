import { test, expect } from '@playwright/test';
import { fixture } from './fixture';
const tv = {
    id: 'e55b4f9d-1865-491e-9585-ae423ad8aca1',
    host: '192.168.1.20',
    port: 4443,
    name: 'Salotto',
};
const remote = {
    version: 1,
    instance: tv.id,
    sequence: 1,
    session: `${tv.id}:1`,
    active: true,
    title: 'Rai 1',
    programme: 'Programma sulla TV',
    playing: true,
    wantsToPlay: true,
    mode: 'LIVE',
    seekable: false,
    position: 60000,
    duration: 0,
    nextEpisode: '',
    transitioning: false,
    options: {
        revision: 1,
        audio: [],
        subtitles: [],
        video: [],
        audioAutomatic: true,
        subtitlesAutomatic: true,
        subtitlesDisabled: true,
        videoAutomatic: true,
        sleepRemaining: 0,
    },
};
test('backup preview allows category selection and reports ignored references', async ({
    page,
}) => {
    const base = await fixture(page);
    let phase = 'ready';
    const restores: Record<string, unknown>[] = [];
    await page.route('**/api/library/backup/**', async (route) => {
        const action = new URL(route.request().url()).pathname.split('/').pop();
        if (action === 'restore') {
            restores.push(route.request().postDataJSON());
            phase = 'restored';
            await route.fulfill({ json: { preferences: base.preferences() } });
            return;
        }
        await route.fulfill({
            json: {
                id: 'backup-fixture',
                phase:
                    action === 'input'
                        ? 'upload'
                        : action === 'backup-fixture'
                          ? 'uploaded'
                          : phase,
                created: 1700000000000,
                favorites: 2,
                series: 1,
                watch: 1,
                sources: [
                    {
                        id: `user_${'a'.repeat(32)}`,
                        name: 'Lista Android sintetica',
                        report: { count: 4 },
                    },
                ],
                result:
                    phase === 'restored'
                        ? { playlists: 0, kept: 0, favorites: 0, watch: 0, skipped: 3 }
                        : undefined,
            },
        });
    });
    await page.locator('[data-route="settings"]').click();
    const backup = page.getByRole('region', { name: 'Backup e ripristino' });
    await backup.getByRole('button', { name: 'Ripristina', exact: true }).click();
    await backup.getByLabel('File Lume').setInputFiles({
        name: 'Lume.lumebackup',
        mimeType: 'application/octet-stream',
        buffer: Buffer.from('encrypted-fixture'),
    });
    await backup.getByLabel('Password del backup').fill('password Android');
    await backup.getByRole('button', { name: 'Verifica backup' }).click();
    await expect(backup.getByRole('heading', { name: 'Contenuto del backup' })).toBeVisible();
    expect(restores).toHaveLength(0);
    for (const name of [
        'Liste personali',
        'Avanzamenti dei video e dell’archivio',
        'Preferenze di riproduzione',
    ])
        await backup.getByRole('checkbox', { name, exact: true }).uncheck();
    await backup.getByRole('button', { name: 'Ripristina selezione' }).click();
    await expect(backup).toContainText(
        '3 riferimenti già presenti o senza un contenuto corrispondente',
    );
    expect(restores[0].categories).toEqual(['PERSONAL']);
});
test('TV pairing waits for approval and remote polling stops when the dialog closes', async ({
    page,
}) => {
    await fixture(page);
    let paired = false;
    let polls = 0;
    let states = 0;
    const commands: Record<string, unknown>[] = [];
    await page.route('**/api/companion/**', async (route) => {
        const action = new URL(route.request().url()).pathname.split('/').pop();
        const body = route.request().postDataJSON();
        let result: unknown = {};
        if (action === 'devices') result = { televisions: paired ? [tv] : [] };
        else if (action === 'discover') result = { televisions: [tv] };
        else if (action === 'pair') {
            expect(body.code).toBe('2345-6789-ABCD-EFGH');
            result = { id: 'pair-fixture', phase: 'waiting', name: tv.name };
        } else if (action === 'status') {
            paired = ++polls > 1;
            result = {
                id: 'pair-fixture',
                phase: paired ? 'paired' : 'waiting',
                name: tv.name,
                television: tv.id,
            };
        } else if (action === 'state') {
            states++;
            result = remote;
        } else if (action === 'command') {
            commands.push(body);
            result = { ...remote, sequence: 2, wantsToPlay: false, playing: false };
        }
        await route.fulfill({ json: result });
    });
    await page.locator('[data-route="settings"]').click();
    const section = page.getByRole('region', { name: 'Collegamento alla TV' });
    await section.getByRole('button', { name: 'Associa una TV', exact: true }).click();
    await section.getByRole('button', { name: 'Cerca TV nella rete' }).click();
    await section.getByRole('button', { name: /Salotto · 192/ }).click();
    await section.getByLabel('Codice di associazione').fill('2345-6789-ABCD-EFGH');
    await section.getByRole('button', { name: 'Associa TV', exact: true }).click();
    await expect(section).toContainText('Conferma l’associazione su Salotto');
    await expect(section.getByRole('button', { name: 'Telecomando', exact: true })).toBeVisible();
    await section.getByRole('button', { name: 'Telecomando', exact: true }).click();
    const dialog = page.getByRole('dialog');
    await expect(dialog.getByRole('heading', { name: 'Telecomando Lume' })).toBeVisible();
    await dialog.getByRole('button', { name: 'Pausa', exact: true }).click();
    await expect(dialog.getByRole('button', { name: 'Riprendi', exact: true })).toBeVisible();
    expect(commands[0]).toMatchObject({ tv: tv.id, session: remote.session, action: 'pause' });
    await dialog.getByRole('button', { name: 'Chiudi', exact: true }).click();
    const count = states;
    await page.waitForTimeout(2300);
    expect(states).toBeLessThanOrEqual(count + 1);
});
test('watch handoff requires a ready result and keeps local playback until TV confirmation', async ({
    page,
}) => {
    const base = await fixture(page);
    const starts: Record<string, unknown>[] = [];
    await page.route('**/api/companion/**', async (route) => {
        const action = new URL(route.request().url()).pathname.split('/').pop();
        let result: unknown = {};
        if (action === 'devices') result = { televisions: [tv] };
        else if (action === 'resolve')
            result = { id: 'watch-fixture', phase: 'ready', title: 'Rai 1', vod: false };
        else if (action === 'start') {
            starts.push(route.request().postDataJSON());
            result = { id: 'watch-fixture', phase: 'started', title: 'Rai 1', vod: false };
        } else if (action === 'state') result = remote;
        await route.fulfill({ json: result });
    });
    await page.locator('[data-play="rai1"]').first().click();
    await page.getByRole('button', { name: 'Guarda sulla TV', exact: true }).click();
    const dialog = page.getByRole('dialog');
    await dialog.getByRole('button', { name: 'Verifica sulla TV' }).click();
    await expect(dialog.getByRole('button', { name: 'Avvia sulla TV' })).toBeVisible();
    expect(starts).toHaveLength(0);
    expect(base.calls.filter((c) => c === '/api/stop')).toHaveLength(0);
    await dialog.getByRole('button', { name: 'Avvia sulla TV' }).click();
    await expect(dialog.getByRole('heading', { name: 'Telecomando Lume' })).toBeVisible();
    expect(starts[0].position).toBe(0);
    await expect.poll(() => base.calls.filter((c) => c === '/api/stop').length).toBe(1);
});
test('hidden channels can be restored and audio language persists through preferences', async ({
    page,
}) => {
    const base = await fixture(page);
    await page.getByRole('button', { name: 'Personalizza Rai 1', exact: true }).click();
    await page.getByRole('dialog').getByRole('button', { name: 'Nascondi canale' }).click();
    await expect(page.locator('.channel-main[data-play="rai1"]')).toHaveCount(0);
    await page.locator('[data-route="settings"]').click();
    await page.getByRole('button', { name: 'Ripristina Rai 1', exact: true }).click();
    await page.getByLabel('Lingua audio preferita').selectOption('en');
    await expect.poll(() => base.preferences().audio_language).toBe('en');
    await page.locator('[data-route="channels"]').click();
    await expect(page.locator('.channel-main[data-play="rai1"]')).toBeVisible();
});

test('playlist transfer requires explicit credential consent and a separate TV import action', async ({
    page,
}) => {
    await fixture(page);
    const source = `user_${'a'.repeat(32)}`;
    const transfers: Record<string, unknown>[] = [];
    const imports: Record<string, unknown>[] = [];
    await page.route('**/api/library', (route) =>
        route.fulfill({
            json: {
                sources: [
                    {
                        id: source,
                        name: 'Lista di prova',
                        enabled: true,
                        revision: 1,
                        has_vod: true,
                        file: true,
                        guide: { updated_at: 0 },
                        report: { count: 4, radio: 0, vod: 2 },
                    },
                ],
            },
        }),
    );
    await page.route('**/api/companion/**', async (route) => {
        const action = new URL(route.request().url()).pathname.split('/').pop();
        let result: unknown = {};
        if (action === 'devices') result = { televisions: [tv] };
        else if (action === 'transfer') {
            transfers.push(route.request().postDataJSON());
            result = {
                id: 'transfer-fixture',
                phase: 'ready',
                preview: {
                    count: 4,
                    sample: ['Film sintetico'],
                    location: 'FILE',
                    allowHttp: false,
                },
            };
        } else if (action === 'import') {
            imports.push(route.request().postDataJSON());
            result = { id: 'transfer-fixture', phase: 'imported' };
        }
        await route.fulfill({ json: result });
    });
    await page.reload();
    await page.locator('[data-route="settings"]').click();
    const section = page.getByRole('region', { name: 'Collegamento alla TV' });
    await section.getByText('Invia una lista alla TV', { exact: true }).click();
    await section.getByLabel('TV di destinazione').selectOption(tv.id);
    await section.getByLabel('Lista da inviare').selectOption(source);
    const send = section.getByRole('button', { name: 'Invia e verifica lista' });
    await expect(send).toBeDisabled();
    expect(transfers).toHaveLength(0);
    await section
        .getByRole('checkbox', {
            name: 'Invia anche le credenziali della lista alla TV selezionata',
        })
        .check();
    await send.click();
    await expect(section.getByRole('heading', { name: 'Lista pronta sulla TV' })).toBeVisible();
    expect(transfers[0]).toEqual({ tv: tv.id, source });
    expect(imports).toHaveLength(0);
    await section.getByLabel('Nome sulla TV').fill('Lista salotto');
    await section.getByRole('button', { name: 'Importa sulla TV' }).click();
    await expect(section).toContainText('Lista importata sulla TV.');
    expect(imports[0]).toEqual({ id: 'transfer-fixture', name: 'Lista salotto' });
});
