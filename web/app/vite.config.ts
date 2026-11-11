import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Fixed names keep the Rust include_bytes! asset map small and auditable.
export default defineConfig(({ mode }) => ({
    plugins: [react()],
    define: {
        'process.env.NODE_ENV': JSON.stringify(
            mode === 'development' ? 'development' : 'production',
        ),
    },
    publicDir: false,
    build: {
        outDir: 'bundle',
        emptyOutDir: true,
        assetsInlineLimit: 0,
        sourcemap: false,
        rolldownOptions: {
            output: {
                entryFileNames: 'app.js',
                assetFileNames: (asset) =>
                    asset.names.some((name) => name.endsWith('.css'))
                        ? 'app.css'
                        : '[name][extname]',
            },
        },
    },
}));
