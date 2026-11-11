import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const ui = process.env.LEGACY_UI ? '.cache/react-baseline' : 'web/app/bundle';
const files = {
    '/': [`${ui}/index.html`, 'text/html'],
    '/app.js': [`${ui}/app.js`, 'text/javascript'],
    '/ui.js': ['.cache/react-baseline/ui.js', 'text/javascript'],
    '/app.css': [`${ui}/app.css`, 'text/css'],
    '/motion.css': ['web/app/motion.css', 'text/css'],
    '/player.js': ['web/player.js', 'text/javascript'],
    '/vendor/shaka-player.compiled.js': [
        'vendor/shaka/dist/shaka-player.compiled.js',
        'text/javascript',
    ],
    ...Object.fromEntries(
        ['logo-light.svg', 'logo-dark.svg', 'symbol.svg'].map((name) => [
            `/assets/${name}`,
            [`web/app/assets/${name}`, 'image/svg+xml'],
        ]),
    ),
    '/favicon.ico': ['web/app/assets/app-icon.png', 'image/png'],
};
// Fixture server has an explicit file map, just like Rust. A missing emitted
// chunk is a 404 rather than an accidental success from Vite's dev server.
createServer(async (request, response) => {
    const file = files[new URL(request.url, 'http://127.0.0.1').pathname];
    if (!file) {
        response.writeHead(404);
        response.end();
        return;
    }
    try {
        const bytes = await readFile(resolve(root, file[0]));
        response.writeHead(200, { 'Content-Type': file[1], 'Cache-Control': 'no-store' });
        response.end(bytes);
    } catch {
        response.writeHead(404);
        response.end();
    }
}).listen(4179, '127.0.0.1');
