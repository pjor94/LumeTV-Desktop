import { useSyncExternalStore } from 'react';

export function createStore<T extends object>(initial: T) {
    let state = initial;
    const listeners = new Set<() => void>();
    return {
        get: () => state,
        set(patch: Partial<T>) {
            state = { ...state, ...patch };
            for (const listener of listeners) listener();
        },
        subscribe(listener: () => void) {
            listeners.add(listener);
            return () => {
                listeners.delete(listener);
            };
        },
    };
}
export function useStore<T extends object>(store: ReturnType<typeof createStore<T>>) {
    return useSyncExternalStore(store.subscribe, store.get);
}
export function useStoreValue<T extends object, V>(
    store: ReturnType<typeof createStore<T>>,
    select: (state: T) => V,
) {
    return useSyncExternalStore(store.subscribe, () => select(store.get()));
}
