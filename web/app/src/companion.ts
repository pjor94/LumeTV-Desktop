import { api } from './api';
import { app } from './app';
export interface Television {
    id: string;
    name: string;
    host: string;
    port: number;
}
export interface CompanionJob {
    id: string;
    phase: string;
    error?: string;
    name?: string;
    television?: string;
    title?: string;
    vod?: boolean;
    received?: number;
    size?: number;
    preview?: { count: number; sample: string[]; allowHttp: boolean; location: string };
}
export function companion<T>(action: string, body: object = {}, signal?: AbortSignal) {
    return api<T>(`/api/companion/${action}`, body, { signal });
}
export function watchOnTv(id: string, position = 0) {
    app.set({ dialog: { kind: 'companion', key: id, position }, dialogOpen: true });
}
export function remoteTv(id: string) {
    app.set({ dialog: { kind: 'remote', key: id }, dialogOpen: true });
}
