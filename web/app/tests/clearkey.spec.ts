import { test, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { fixture, channels } from './fixture';

// Generated test pattern, encrypted with public synthetic test keys. No provider data.
const clearKeys = { '00112233445566778899aabbccddeeff': '000102030405060708090a0b0c0d0e0f' };
test('real Shaka decodes explicit ClearKey and resets keys when switching to Widevine', async ({
    page,
}) => {
    await fixture(page, { realPlayer: true });
    await page.evaluate(() => {
        const ActualPlayer = (window as any).shaka.Player;
        (window as any).shaka.Player = new Proxy(ActualPlayer, {
            construct(target, args) {
                const player = Reflect.construct(target, args) as object;
                (window as any).realTestPlayer = player;
                return player;
            },
        });
    });
    const errors: number[] = [];
    await page.route('**/api/event', async (route) => {
        const event = route.request().postDataJSON();
        if (event.kind === 'error') errors.push(Number(event.code));
        await route.fulfill({ json: {} });
    });
    await page.route('**/api/play', (route) =>
        route.fulfill({
            json: {
                playback_id: 'encrypted-fixture',
                channel: channels[0],
                stream: {
                    url: 'https://fixture.invalid/clearkey/manifest.mpd',
                    mime_type: 'application/dash+xml',
                    mode: 'vod',
                    duration: 3,
                    drm:
                        route.request().postDataJSON().channel_id === 'rai1'
                            ? { key_system: 'org.w3.clearkey', clear_keys: clearKeys }
                            : { key_system: 'com.widevine.alpha' },
                },
            },
        }),
    );
    await page.route('**/api/fetch', async (route) => {
        const request = route.request().postDataJSON();
        if (request.kind === 'license') {
            await route.fulfill({ status: 400, json: { error: 'unexpected_license' } });
            return;
        }
        const name = new URL(request.url).pathname.split('/').at(-1)!;
        if (!/^(manifest\.mpd|init-stream0\.m4s|chunk-stream0-\d{5}\.m4s)$/.test(name))
            throw new Error('Unexpected fixture request');
        await route.fulfill({
            body: name.endsWith('.mpd')
                ? readFileSync(new URL(`./media/clearkey/${name}`, import.meta.url))
                : Buffer.from(
                      readFileSync(
                          new URL(`./media/clearkey/${name}.base64`, import.meta.url),
                          'utf8',
                      ).trim(),
                      'base64',
                  ),
            contentType: name.endsWith('.mpd') ? 'application/dash+xml' : 'video/mp4',
            headers: { 'X-Lume-Final-Url': request.url },
        });
    });
    await page.locator('[data-play="rai1"]').first().click();
    await expect
        .poll(
            () => page.locator('#video').evaluate((video: HTMLVideoElement) => video.currentTime),
            { timeout: 20000 },
        )
        .toBeGreaterThan(1);
    expect(
        await page
            .locator('#video')
            .evaluate(
                (video: HTMLVideoElement) => video.getVideoPlaybackQuality().totalVideoFrames,
            ),
    ).toBeGreaterThan(5);
    expect(errors).toEqual([]);
    await page.locator('[data-route="channels"]').click();
    await page.locator('[data-play="canale5"]').first().click();
    await expect
        .poll(() =>
            page.evaluate(() => {
                const drm = (window as any).realTestPlayer.getConfiguration().drm;
                return { clearKeys: drm.clearKeys, servers: drm.servers };
            }),
        )
        .toEqual({ clearKeys: {}, servers: { 'com.widevine.alpha': 'lume-license:license' } });
});
