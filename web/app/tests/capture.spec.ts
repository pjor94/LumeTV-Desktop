import { test, expect } from '@playwright/test';
import { fixture } from './fixture';
import { mkdir } from 'node:fs/promises';

test('capture the incumbent interface for the migration comparison', async ({ page }) => {
    test.skip(!process.env.LUME_CAPTURE, 'Local visual comparison only');
    await fixture(page);
    await expect(page.locator('.channel-grid .channel-card').first()).toBeVisible();
    const directory = `../../.cache/react-${process.env.LEGACY_UI ? 'before' : 'after'}`;
    await mkdir(directory, { recursive: true });
    for (const theme of ['dark', 'light']) {
        if (theme === 'light') {
            await page.locator('[data-route=settings]').click();
            await page.locator('[data-theme=light]').click();
        }
        for (const width of [1280, 820]) {
            await page.setViewportSize({ width, height: 820 });
            for (const route of ['channels', 'discover', 'guide', 'favorites', 'settings']) {
                await page.locator(`.nav-item[data-route=${route}]`).click();
                await page.screenshot({
                    path: `${directory}/${theme}-${width}-${route}.png`,
                    animations: 'disabled',
                    mask: [page.locator('#search-shortcut')],
                });
            }
        }
    }
    await page.setViewportSize({ width: 1280, height: 820 });
    await page.locator('.nav-item[data-route=channels]').click();
    await page.locator('.channel-main[data-play=rai1]').click();
    await expect(page.locator('#player-overlay')).toBeHidden();
    await page.screenshot({
        path: `${directory}/watch.png`,
        animations: 'disabled',
        mask: [page.locator('#search-shortcut')],
    });
    await page.locator('.nav-item[data-route=channels]').click();
    await page.screenshot({
        path: `${directory}/mini.png`,
        animations: 'disabled',
        mask: [page.locator('#search-shortcut')],
    });
});
