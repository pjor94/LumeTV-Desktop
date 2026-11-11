import { test, expect } from '@playwright/test';
import { fixture, channels } from './fixture';
import type { Channel } from '../src/types';

const radios: Channel[] = Array.from({ length: 133 }, (_, i) => ({
    kind: 'radio',
    id: `radio-${i}`,
    name: i === 0 ? 'Radio Città' : `Radio ${i}`,
    group: i % 2 ? 'Roma' : 'Milano',
    number: 9999,
    provider: 'direct',
    color: '#99caff',
}));
test('pausing an offline radio cancels automatic recovery', async ({ page }) => {
    const data = await fixture(page, { radioChannels: radios, continuousRadio: true });
    await page.locator('[data-route=radio]').click();
    await page.locator('.radio-main[data-play=radio-0]').click();
    await expect(page.locator('#player-overlay')).toBeHidden();
    await page.evaluate(() => {
        window.dispatchEvent(new Event('offline'));
        document.querySelector('video')!.dispatchEvent(new Event('ended'));
    });
    await expect(page.locator('#player-status')).toHaveText('Connessione assente.');
    await page.locator('#play-pause').click();
    await expect(page.locator('#player-overlay')).toBeHidden();
    await page.evaluate(() => window.dispatchEvent(new Event('online')));
    await expect(page.locator('#play-pause')).toHaveAttribute('title', 'Riprendi · Spazio');
    expect(data.plays).toEqual(['radio-0']);
    await page.locator('#play-pause').click();
    await expect.poll(() => data.plays.length).toBe(2);
});
test('radio search, pagination and favorites stay separate from TV and guide', async ({ page }) => {
    await fixture(page, { radioChannels: radios });
    await expect(page.locator('.channel-grid .channel-card')).toHaveCount(channels.length);
    await page.locator('[data-route=radio]').click();
    await expect(page.locator('.radio-card')).toHaveCount(48);
    await page.getByRole('button', { name: 'Successive' }).click();
    await expect(page.locator('.radio-pagination')).toContainText('49–96');
    await page.locator('#search').fill('citta');
    await expect(page.locator('.radio-card')).toHaveCount(1);
    await page.locator('.radio-card .favorite-button').click();
    await page.locator('#search').fill('');
    await page.getByRole('button', { name: 'Preferite', exact: true }).click();
    await expect(page.locator('.radio-card')).toHaveCount(1);
    await page.locator('.radio-main[data-play=radio-0]').click();
    await expect(page.locator('#player-overlay')).toBeHidden();
    await page.locator('[data-route=radio]').click();
    await expect(page.getByRole('button', { name: 'Preferite', exact: true })).toHaveAttribute(
        'aria-pressed',
        'true',
    );
    await expect(page.locator('.radio-card')).toHaveCount(1);
    await page.locator('[data-route=guide]').click();
    await expect(page.locator('#page-content [data-play=radio-0]')).toHaveCount(0);
});

test('capture radio catalog and listening layouts', async ({ page }) => {
    test.skip(!process.env.LUME_CAPTURE, 'Local visual comparison only');
    await fixture(page, {
        radioChannels: radios,
        continuousRadio: true,
        radioTitle: 'Artista - Brano',
    });
    for (const theme of ['dark', 'light']) {
        await page.locator('[data-route=settings]').click();
        await page.locator(`button[data-theme=${theme}]`).click();
        for (const width of [1280, 820]) {
            await page.setViewportSize({ width, height: 820 });
            await page.locator('[data-route=radio]').click();
            await page.screenshot({
                path: `../../.cache/radio-${theme}-${width}-catalog.png`,
                animations: 'disabled',
            });
            expect(
                await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),
            ).toBe(true);
            await page.locator('.radio-main[data-play=radio-0]').click();
            await expect(page.locator('#player-overlay')).toBeHidden();
            await expect(page.locator('.audio-screen p')).toHaveText('Artista - Brano');
            await page.screenshot({
                path: `../../.cache/radio-${theme}-${width}-listen.png`,
                animations: 'disabled',
            });
            await page.locator('[data-route=radio]').click();
            await page.screenshot({
                path: `../../.cache/radio-${theme}-${width}-mini.png`,
                animations: 'disabled',
            });
            await page.locator('#mini-close').click();
        }
    }
});
test('continuous radio uses the persistent media element, shows metadata and stops cleanly', async ({
    page,
}) => {
    const data = await fixture(page, {
        radioChannels: radios,
        continuousRadio: true,
        radioTitle: 'Artista - Brano',
    });
    await page.locator('[data-route=radio]').click();
    await page.locator('.radio-main[data-play=radio-0]').click();
    await expect(page.locator('#player-overlay')).toBeHidden();
    await expect(page.locator('#player-dock')).toHaveClass(/radio-mode/);
    await expect(page.locator('.audio-screen p')).toHaveText('Artista - Brano');
    const video = await page.locator('#video').elementHandle();
    await page.locator('[data-route=channels]').click();
    await expect(page.locator('#mini-expand')).toBeVisible();
    expect(await video!.evaluate((node) => node === document.querySelector('video'))).toBe(true);
    await page.locator('#mini-close').click();
    await expect(page.locator('#mini-expand')).toBeHidden();
    expect(data.calls).toContain('/api/stop');
});
test('radio and TV channel changes remain within their respective catalogues', async ({ page }) => {
    const data = await fixture(page, { radioChannels: radios });
    await page.locator('[data-route=radio]').click();
    await page.locator('.radio-main[data-play=radio-0]').click();
    await expect(page.locator('#player-overlay')).toBeHidden();
    await page.evaluate(() =>
        (
            window as unknown as { testPlayer: { emit(name: string, event: unknown): void } }
        ).testPlayer.emit('metadata', { payload: { key: 'TIT2', data: 'Titolo HLS' } }),
    );
    await expect(page.locator('.audio-screen p')).toHaveText('Titolo HLS');
    await page.locator('#player-dock').focus();
    await page.keyboard.press('ArrowDown');
    await expect.poll(() => data.plays.at(-1)).toBe('radio-1');
    await page.locator('[data-route=channels]').click();
    await page.locator('.channel-main[data-play=rai1]').click();
    await expect(page.locator('#player-overlay')).toBeHidden();
    await page.locator('#player-dock').focus();
    await page.keyboard.press('ArrowDown');
    await expect.poll(() => data.plays.at(-1)).toBe('rai2');
});
