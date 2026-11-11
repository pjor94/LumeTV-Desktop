import type { Track } from './types';

export interface Capabilities extends Record<string, unknown> {
    secure_context: boolean;
    eme_available: boolean;
    widevine_available: boolean;
    avc_mse_supported: boolean;
    aac_mse_supported: boolean;
}
export interface ShakaPlayer {
    attach(video: HTMLVideoElement): Promise<void>;
    unload(): Promise<void>;
    destroy(): Promise<void>;
    load(url: string, start?: number, mime?: string): Promise<void>;
    configure(configuration: Record<string, unknown>): void;
    addEventListener(
        name: string,
        listener: (event: { detail?: unknown; buffering?: boolean }) => void,
    ): void;
    getNetworkingEngine(): {
        registerRequestFilter(filter: (kind: unknown, request: { uris: string[] }) => void): void;
    };
    getKeyStatuses(): unknown;
    getVariantTracks(): Track[];
    getTextTracks(): Track[];
    isTextTrackVisible(): boolean;
    setTextTrackVisibility(value: boolean): void;
    selectAudioLanguage(language: string): void;
    selectTextTrack(track: Track): void;
    seekRange(): { end: number };
}
declare global {
    interface Window {
        shaka: {
            Player: new () => ShakaPlayer;
            polyfill: { installAll(): void };
            net: { NetworkingEngine: { registerScheme(scheme: string, plugin: unknown): void } };
            log?: { setLevel(level: number): void; Level: { NONE: number } };
        };
        LumePlayback: {
            makeRelayPlugin(
                shaka: Window['shaka'],
                fetch: (path: string, options: RequestInit) => Promise<Response>,
                kind: string,
            ): unknown;
            probeCapabilities(video: HTMLVideoElement): Promise<Capabilities>;
            sanitizedError(error: unknown): Record<string, unknown>;
            safeKeyStatuses(statuses: unknown): string[];
        };
    }
}
