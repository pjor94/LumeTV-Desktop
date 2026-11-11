import { test, expect } from '@playwright/test';
import { fixture, updateStatus } from './fixture';
import type { UpdateStatus } from '../src/updates';

const available: Partial<UpdateStatus> = {
    phase: 'available',
    available_version: '0.2.0',
    total: 104857600,
    notes: 'Riproduzione e guida TV aggiornate.',
};

test('download progress, cancellation and verification errors remain actionable', async ({
    page,
}) => {
    let status = { ...updateStatus, ...available };
    const data = await fixture(page, {
        updates: (path) => {
            if (path.endsWith('/download'))
                status = { ...status, phase: 'downloading', downloaded: 26214400, error: null };
            if (path.endsWith('/cancel')) status = { ...status, phase: 'available', downloaded: 0 };
            return status;
        },
    });
    await page.locator('[data-route=settings]').click();
    await page.getByRole('button', { name: 'Scarica aggiornamento', exact: false }).click();
    await expect(page.getByRole('button', { name: 'Annulla download' })).toBeFocused();
    await expect(page.getByRole('progressbar', { name: 'Download aggiornamento' })).toHaveAttribute(
        'value',
        '25',
    );
    await expect(page.locator('.update-transfer')).toHaveText('25 MB di 100 MB25%');
    await expect(page.getByRole('button', { name: 'Installa e riavvia' })).toHaveCount(0);
    await expect(page.getByRole('switch', { name: 'Includi versioni di prova' })).toBeDisabled();
    if (process.env.LUME_CAPTURE) {
        for (const width of [1280, 820]) {
            await page.setViewportSize({ width, height: 900 });
            for (const theme of ['dark', 'light']) {
                await page.locator(`button[data-theme=${theme}]`).click();
                await page.locator('.update-section').scrollIntoViewIfNeeded();
                await page.screenshot({ path: `../../.cache/updates-${theme}-${width}.png` });
                expect(
                    await page
                        .locator('.update-section')
                        .evaluate((node) => node.scrollWidth <= node.clientWidth),
                ).toBe(true);
            }
        }
    }
    await page.getByRole('button', { name: 'Annulla download' }).click();
    await expect(page.getByRole('progressbar')).toHaveCount(0);
    await expect(
        page.getByRole('button', { name: 'Scarica aggiornamento', exact: false }),
    ).toBeVisible();
    status = { ...status, error: 'checksum' };
    await page.getByRole('button', { name: 'Controlla', exact: true }).click();
    await expect(page.locator('.update-message')).toContainText('Scarica di nuovo il pacchetto');
    expect(data.calls).toContain('/api/updates/cancel');
    expect(data.calls).not.toContain('/api/updates/install');
});

test('installing a verified update stops playback before requesting restart', async ({ page }) => {
    const data = await fixture(page, {
        updates: { ...available, phase: 'ready', downloaded: 104857600 },
    });
    await page.locator('.channel-main[data-play=rai1]').click();
    await expect(page.locator('#player-overlay')).toBeHidden();
    await page.locator('[data-route=settings]').click();
    await expect(page.locator('#mini-expand')).toBeVisible();
    await page.getByRole('button', { name: 'Installa e riavvia' }).click();
    await expect(page.locator('#mini-expand')).toBeHidden();
    await expect.poll(() => data.calls.includes('/api/updates/install')).toBe(true);
    expect(data.calls.indexOf('/api/stop')).toBeLessThan(
        data.calls.indexOf('/api/updates/install'),
    );
});

test('a background update never opens a dialog or toast over live TV', async ({ page }) => {
    let ready = false;
    await fixture(page, { updates: () => (ready ? available : { phase: 'checking' }) });
    await page.locator('.channel-main[data-play=rai1]').click();
    await expect(page.locator('#player-overlay')).toBeHidden();
    ready = true;
    await expect(page.getByRole('img', { name: 'Aggiornamento disponibile' })).toHaveCount(1);
    await expect(page.locator('.toast')).toHaveCount(0);
    await expect(page.locator('dialog[open]')).toHaveCount(0);
    await expect(page.locator('body')).toHaveAttribute('data-page', 'watch');
});

test('the system package action explains that installation continues outside the app', async ({
    page,
}) => {
    const data = await fixture(page, {
        updates: { ...available, phase: 'ready', install_mode: 'system' },
    });
    await page.locator('[data-route=settings]').click();
    await expect(page.locator('.update-detail')).toContainText('gestore dei pacchetti');
    await page.getByRole('button', { name: 'Apri installazione' }).click();
    await expect.poll(() => data.calls.includes('/api/updates/install')).toBe(true);
});

test('automatic checks and preview preferences can be changed independently', async ({ page }) => {
    let status = {
        ...updateStatus,
        settings: { automatic: false, preview: true },
        next_check_at: 0,
    };
    const data = await fixture(page, {
        updates: (path, body) => {
            if (path.endsWith('/settings'))
                status = { ...status, settings: { ...status.settings, ...body } };
            return status;
        },
    });
    await page.locator('[data-route=settings]').click();
    expect(data.calls).not.toContain('/api/updates/check');
    await page.getByRole('switch', { name: 'Includi versioni di prova' }).click();
    await expect(page.getByRole('switch', { name: 'Includi versioni di prova' })).toHaveAttribute(
        'aria-checked',
        'false',
    );
    await expect(
        page.getByRole('switch', { name: 'Controlla automaticamente gli aggiornamenti' }),
    ).toHaveAttribute('aria-checked', 'false');
    await page.getByRole('button', { name: 'Controlla', exact: true }).click();
    expect(data.calls).toContain('/api/updates/check');
});
