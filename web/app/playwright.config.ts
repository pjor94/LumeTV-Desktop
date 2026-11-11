import { defineConfig } from '@playwright/test';

export default defineConfig({
    testDir: './tests',
    testMatch: '**/*.spec.ts',
    workers: 1,
    use: {
        baseURL: 'http://127.0.0.1:4179',
        viewport: { width: 1280, height: 820 },
        locale: 'it-IT',
        timezoneId: 'Europe/Rome',
        launchOptions: {
            ...(process.env.LUME_TEST_CHROME ? { channel: 'chrome' } : {}),
            args: ['--autoplay-policy=no-user-gesture-required'],
        },
    },
    webServer: {
        command: 'node tests/server.mjs',
        url: 'http://127.0.0.1:4179',
        reuseExistingServer: false,
    },
});
