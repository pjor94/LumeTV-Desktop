import { test, expect } from '@playwright/test';
import { fixture } from './fixture';
import type { WidevineStatus } from '../src/types';

test('first launch prepares Widevine before exposing any channel or player', async ({ page }) => {
    let phase: WidevineStatus['phase'] = 'downloading';
    const data = await fixture(page, { startupWidevine: false, status: () => ({ phase }) });
    await expect(page.locator('.startup-screen')).toBeVisible();
    await expect(page.locator('.startup-status h2')).toHaveText('Download di Widevine in corso.');
    await expect(page.locator('.sidebar')).toHaveCount(0);
    await expect(page.locator('video')).toHaveCount(0);
    await page.keyboard.press('Meta+1');
    await expect(page.locator('.sidebar')).toHaveCount(0);
    expect(data.calls).toContain('/api/widevine/update');
    expect(data.plays).toEqual([]);
    if (process.env.LUME_CAPTURE)
        await page.screenshot({ path: '../../.cache/react-startup-dark.png' });
    phase = 'installing';
    await expect(page.locator('.startup-status h2')).toHaveText('Installazione di Widevine.');
    phase = 'installed';
    await expect(page.locator('.startup-status h2')).toHaveText('Verifica dei canali protetti.');
    await expect(page.locator('.sidebar')).toHaveCount(0);
    await page.evaluate('testStartupWidevineReady = true');
    await expect(page.locator('.sidebar')).toBeVisible();
    expect(data.plays).toEqual([]);
    expect(await page.evaluate('testPlayer.loads')).toBe(0);
});

test('an existing usable component goes straight to the catalog without a download', async ({
    page,
}) => {
    const data = await fixture(page);
    await expect(page.locator('.channel-grid')).toBeVisible();
    expect(data.calls.some((path) => path.startsWith('/api/widevine'))).toBe(false);
});

test('first-launch errors offer retry and never unlock the catalog early', async ({ page }) => {
    let phase: WidevineStatus['phase'] = 'error';
    const data = await fixture(page, {
        startupWidevine: false,
        status: () => ({ phase, error: phase === 'error' ? 'check_error' : null }),
    });
    await expect(page.getByRole('button', { name: 'Riprova', exact: true })).toBeVisible();
    await expect(page.getByRole('progressbar')).toHaveCount(0);
    await expect(page.locator('.sidebar')).toHaveCount(0);
    phase = 'downloading';
    await page.getByRole('button', { name: 'Riprova', exact: true }).click();
    await expect(page.getByRole('progressbar')).toBeVisible();
    expect(data.plays).toEqual([]);
    await page.evaluate('testStartupWidevineReady = true');
    await expect(page.locator('.sidebar')).toBeVisible();
});

test('Linux installation keeps the startup screen until reopening with a usable component', async ({
    page,
}) => {
    let reopened = false;
    const data = await fixture(page, {
        startupWidevine: () => reopened,
        status: { phase: 'installed', restart_required: true },
    });
    await expect(page.locator('.startup-status h2')).toHaveText('Widevine è installato.');
    await expect(page.locator('.startup-status')).toContainText(
        'chiudi completamente Lume e riaprila',
    );
    await expect(page.getByRole('progressbar')).toHaveCount(0);
    await expect(page.locator('.sidebar')).toHaveCount(0);
    expect(data.plays).toEqual([]);
    reopened = true;
    await page.reload();
    await expect(page.locator('.sidebar')).toBeVisible();
});

test('startup respects the saved light theme and reduced motion', async ({ page }) => {
    await page.emulateMedia({ reducedMotion: 'reduce' });
    const data = await fixture(page, { startupWidevine: false });
    data.preferences().theme = 'light';
    await page.reload();
    await expect(page.locator('.startup-status h2')).toHaveText('Download di Widevine in corso.');
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
    expect(
        await page
            .locator('.startup-progress span')
            .evaluate((element) => element.getAnimations().length),
    ).toBe(0);
    if (process.env.LUME_CAPTURE)
        await page.screenshot({ path: '../../.cache/react-startup-light.png' });
});
