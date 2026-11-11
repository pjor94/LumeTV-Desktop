import { test, expect } from '@playwright/test';
import { fixture, channels } from './fixture';
import type { WidevineStatus } from '../src/types';

test('catalog, filters and search retain focus and render guide metadata as text', async ({
    page,
}) => {
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await fixture(page);
    await expect(page.locator('.channel-grid .channel-card')).toHaveCount(channels.length);
    await page.locator('[data-provider=mediaset]').click();
    await expect(page.locator('.channel-grid .channel-card')).toHaveCount(
        channels.filter((c) => c.provider === 'mediaset').length,
    );
    const search = page.locator('#search');
    await search.fill('La7');
    await expect(page.locator('.channel-grid .channel-card')).toHaveCount(2);
    await expect(search).toBeFocused();
    await search.fill('<img src=x onerror=alert(1)>');
    await expect(page.locator('#page-content .page-heading p')).toContainText(
        '<img src=x onerror=alert(1)>',
    );
    await expect(page.locator('#page-content img[src=x]')).toHaveCount(0);
    await page.keyboard.press('Escape');
    await expect(search).toHaveValue('');
    expect(errors).toEqual([]);
    expect(new URL(page.url()).hash).toBe('');
});

test('theme and quick favorite edits survive reload and preserve DOM identity', async ({
    page,
}) => {
    const data = await fixture(page);
    const card = await page.locator('[data-key=channel-rai1]').elementHandle();
    await page.locator('.channel-card [data-favorite=rai1]').click();
    await page.locator('.channel-card [data-favorite=rai2]').click();
    await expect(page.locator('.channel-card [data-favorite=rai2]')).toHaveAttribute(
        'aria-pressed',
        'true',
    );
    expect(await card!.evaluate((node) => node.isConnected)).toBe(true);
    await page.locator('.nav-item[data-route=settings]').click();
    await page.locator('[data-theme=light]').click();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
    await page.reload();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
    await expect(page.locator('.channel-card [data-favorite=rai2]')).toHaveAttribute(
        'aria-pressed',
        'true',
    );
    expect(data.preferences().favorites).toEqual(['canale5', 'rai2']);
});

test('one video and one player survive navigation, mini mode and fullscreen', async ({ page }) => {
    await fixture(page);
    const video = await page.locator('#video').elementHandle();
    await page.locator('.channel-main[data-play=rai1]').click();
    await expect(page.locator('#player-overlay')).toBeHidden();
    for (const route of ['guide', 'settings', 'channels'])
        await page.locator(`.nav-item[data-route=${route}]`).click();
    await expect(page.locator('#mini-expand')).toBeVisible();
    expect(
        await video!.evaluate(
            (node) => node.isConnected && node === document.querySelector('video'),
        ),
    ).toBe(true);
    await page.locator('#mini-expand').click();
    await page.locator('#fullscreen').click();
    await expect
        .poll(() => page.evaluate(() => document.fullscreenElement?.id))
        .toBe('player-dock');
    await page.keyboard.press('Escape');
    await expect.poll(() => page.evaluate(() => document.fullscreenElement)).toBeNull();
    expect(await page.evaluate('testPlayer')).toEqual({
        attachments: 1,
        loads: 1,
        unloads: 1,
        destroyed: 0,
    });
    expect(await video!.evaluate((node) => node.isConnected)).toBe(true);
});

test('removing favorites with the keyboard passes focus to the next available action', async ({
    page,
}) => {
    await fixture(page);
    await page.locator('.nav-item[data-route=favorites]').click();
    await page.locator('.channel-card [data-favorite=rai1]').focus();
    await page.keyboard.press('Enter');
    await expect(page.locator('.channel-card [data-favorite=canale5]')).toBeFocused();
    await page.keyboard.press('Enter');
    await expect(page.locator('.empty-state button')).toBeFocused();
});

test('fast channel changes discard an older response without interrupting the newer channel', async ({
    page,
}) => {
    const data = await fixture(page, { delayPlay: true });
    await page.locator('.channel-main[data-play=rai1]').click();
    await page.locator('#watch-browser [data-play=canale5]').click();
    await expect(page.locator('#player-channel-label')).toHaveText('Canale 5');
    await expect(page.locator('#player-overlay')).toBeHidden();
    await page.waitForTimeout(700);
    expect(data.plays).toEqual(['rai1', 'canale5']);
    expect(await page.evaluate('testPlayer.loads')).toBe(1);
    await expect(page.locator('#player-channel-label')).toHaveText('Canale 5');
});

test('guide details, reminders and native dialog keyboard focus work', async ({ page }) => {
    await fixture(page);
    await page.locator('.nav-item[data-route=guide]').click();
    await page.locator('.programme-block').nth(1).click();
    await expect(page.locator('#details-dialog')).toBeVisible();
    await page.locator('[data-remind]').click();
    await expect(page.locator('[data-remind]')).toHaveAttribute('aria-pressed', 'true');
    await page.keyboard.press('Escape');
    await expect(page.locator('#details-dialog')).not.toBeVisible();
    await page.locator('#reminders-button').click();
    await expect(page.locator('.reminder-item')).toHaveCount(1);
    await page.locator('[data-remove-reminder]').click();
    await expect(page.locator('.reminder-item')).toHaveCount(0);
    await page.keyboard.press('Escape');
    await page.locator('#shortcuts-button').click();
    await expect(page.locator('.shortcut-list')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.locator('#shortcuts-button')).toBeFocused();
});

test('interrupting the mini transition preserves picture geometry and clears animation effects', async ({
    page,
}) => {
    await fixture(page);
    await page.locator('.channel-main[data-play=rai1]').click();
    await expect(page.locator('#player-overlay')).toBeHidden();
    await page.locator('.nav-item[data-route=channels]').click();
    await page.locator('#mini-expand').click({ force: true });
    await expect
        .poll(() => page.locator('#player-dock').getAttribute('data-dock-moving'))
        .toBeNull();
    const geometry = await page.locator('#video').evaluate((video) => {
        const rect = video.getBoundingClientRect();
        return {
            ratio: rect.width / rect.height,
            animations: video.getAnimations().length,
            willChange: video.style.willChange,
        };
    });
    expect(geometry.ratio).toBeCloseTo(16 / 9, 2);
    expect(geometry.animations).toBe(0);
    expect(geometry.willChange).toBe('');
});

test('Widevine shows actual phases without a made-up percentage and starts after EME succeeds', async ({
    page,
}) => {
    let phase: WidevineStatus['phase'] = 'downloading';
    await fixture(page, { widevine: false, status: () => ({ phase }) });
    await page.locator('.channel-main[data-play=canale5]').click();
    const bar = page.getByRole('progressbar', { name: 'Preparazione di Widevine' });
    await expect(bar).toBeVisible();
    await expect(bar).not.toHaveAttribute('aria-valuenow');
    await expect(page.locator('#player-status')).toHaveText('Download di Widevine in corso.');
    if (process.env.LUME_CAPTURE)
        await page.screenshot({ path: '../../.cache/react-widevine.png' });
    phase = 'installing';
    await expect(page.locator('#player-status')).toHaveText('Installazione di Widevine.');
    phase = 'installed';
    await expect(page.locator('#player-status')).toHaveText('Verifica dei canali protetti.');
    expect(await page.evaluate('testPlayer.loads')).toBe(0);
    await page.evaluate('testWidevineReady = true');
    await expect(page.locator('#player-overlay')).toBeHidden();
    expect(await page.evaluate('testPlayer.loads')).toBe(1);
});

test('Linux new installation gives explicit restart guidance and allows switching to Rai', async ({
    page,
}) => {
    await fixture(page, {
        widevine: false,
        status: { phase: 'installed', restart_required: true },
    });
    await page.locator('.channel-main[data-play=canale5]').click();
    await expect(page.locator('#player-status')).toHaveText('Widevine è installato. Riapri Lume.');
    await expect(page.locator('#player-status-detail')).toContainText('Chiudi completamente Lume');
    await expect(page.getByRole('progressbar')).toBeHidden();
    await page.locator('#watch-browser [data-play=rai1]').click();
    await expect(page.locator('#player-overlay')).toBeHidden();
    await expect(page.locator('.widevine-setup')).toHaveCount(0);
});

test('a Widevine error stops the loader and retry does not create another player', async ({
    page,
}) => {
    let phase: WidevineStatus['phase'] = 'error';
    await fixture(page, {
        widevine: false,
        status: () => ({ phase, error: phase === 'error' ? 'check_error' : null }),
    });
    await page.locator('.channel-main[data-play=la7]').click();
    await expect(page.locator('#player-retry')).toBeVisible();
    await expect(page.locator('#player-status')).toHaveText('Download di Widevine non riuscito.');
    phase = 'downloading';
    await page.locator('#player-retry').click();
    await expect(page.getByRole('progressbar')).toBeVisible();
    expect(await page.evaluate('testPlayer.attachments')).toBe(1);
});

test('reduced motion retains progress information without a moving bar', async ({ page }) => {
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await fixture(page, { widevine: false });
    await page.locator('.channel-main[data-play=canale5]').click();
    await expect(page.getByRole('progressbar')).toBeVisible();
    expect(
        await page
            .locator('.widevine-progress span')
            .evaluate((element) => element.getAnimations().length),
    ).toBe(0);
    await expect(page.locator('.widevine-elapsed')).toContainText('Tempo trascorso:');
});
