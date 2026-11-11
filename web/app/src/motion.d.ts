export interface Rect {
    x: number;
    y: number;
    width: number;
    height: number;
}
export interface DockRect {
    dock: Rect;
    video: Rect;
}
interface Motion {
    initialize(document: Document): void;
    canMove(): boolean;
    reduced(): boolean;
    openDialog(dialog: HTMLDialogElement): void;
    closeDialog(dialog: HTMLDialogElement, immediate?: boolean): void;
    setOptions(element: HTMLElement, visible: boolean, immediate?: boolean): void;
    confirmFavorite(button: HTMLButtonElement): void;
    captureDock(dock: HTMLElement): DockRect | null;
    moveDock(dock: HTMLElement, before: DockRect | null): void;
    stopDock(dock: HTMLElement): void;
    dockFrames(before: DockRect, after: DockRect): { dock: Keyframe[]; video: Keyframe[] } | null;
    introduceBrand(element: Element | null): void;
}
export function createMotion(global: unknown): Motion;
export const motion: Motion;
export const dockFrames: Motion['dockFrames'];
