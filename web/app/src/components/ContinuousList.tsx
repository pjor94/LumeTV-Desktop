import {
    useEffect,
    useLayoutEffect,
    useRef,
    useState,
    useSyncExternalStore,
    type ReactNode,
} from 'react';
import { ContinuousData, readSession, writeSession, type LibraryPage } from '../personal';

interface Props<T> {
    context: string;
    revision?: string;
    label: string;
    blockSize?: number;
    height?: number;
    rowHeight?: number;
    layout?: 'list' | 'channels' | 'radio' | 'videos';
    itemNames?: [string, string];
    active?: boolean;
    load(offset: number, signal: AbortSignal): Promise<LibraryPage<T>>;
    refreshInterval?: number;
    render(row: T, position: number): ReactNode;
    empty?: ReactNode;
}
export function ContinuousList<T>({
    context,
    revision = '',
    label,
    blockSize = 200,
    height = 520,
    rowHeight = 76,
    layout = 'list',
    itemNames = ['risultato', 'risultati'],
    active = true,
    refreshInterval = 0,
    load,
    render,
    empty,
}: Props<T>) {
    const loader = useRef(load);
    loader.current = load;
    const data = useRef<ContinuousData<T> | null>(null);
    if (!data.current) data.current = new ContinuousData(blockSize, (o, s) => loader.current(o, s));
    const controller = data.current;
    const dataRevision = useSyncExternalStore(controller.subscribe, controller.snapshot);
    const viewport = useRef<HTMLDivElement>(null);
    const canvas = useRef<HTMLDivElement>(null);
    const scrollRoot = useRef<HTMLElement | null>(null);
    const grid = layout !== 'list';
    const [scroll, setScroll] = useState(0);
    const [visibleHeight, setVisibleHeight] = useState(height);
    const [geometry, setGeometry] = useState({
        columns: 1,
        itemHeight: grid ? 183 : rowHeight,
        gap: 0,
    });
    const columns = grid ? geometry.columns : 1;
    const itemHeight = grid ? geometry.itemHeight : rowHeight;
    const pitch = itemHeight + (grid ? geometry.gap : 0);
    const rows = Math.ceil(controller.total / columns);
    const logicalHeight = Math.max(0, rows * pitch - (grid ? geometry.gap : 0));
    const physicalHeight = Math.min(logicalHeight, 8_000_000);
    const scale = Math.max(
        1,
        (logicalHeight - visibleHeight) / Math.max(1, physicalHeight - visibleHeight),
    );
    const firstRow = Math.max(
        0,
        Math.min(Math.floor((scroll * scale) / pitch), Math.max(0, rows - 1)),
    );
    const first = firstRow * columns;
    const start = Math.max(0, firstRow - 2) * columns;
    const end = Math.min(
        controller.total,
        (firstRow + Math.ceil(visibleHeight / pitch) + 4) * columns,
    );
    const readScroll = useRef(() => {});
    const measure = useRef(() => {});
    const hidden = useRef(false);
    const canvasOffset = () => {
        const root = scrollRoot.current;
        return grid && root && canvas.current
            ? canvas.current.getBoundingClientRect().top -
                  root.getBoundingClientRect().top +
                  root.scrollTop
            : 0;
    };
    readScroll.current = () => {
        const root = scrollRoot.current;
        if (!active || hidden.current || (grid && !canvas.current?.clientWidth)) return;
        if (root)
            setScroll(
                Math.max(
                    0,
                    Math.min(
                        root.scrollTop - canvasOffset(),
                        Math.max(0, physicalHeight - root.clientHeight),
                    ),
                ),
            );
    };
    measure.current = () => {
        const root = scrollRoot.current;
        if (!root || !canvas.current) return;
        if (!active || (grid && !canvas.current.clientWidth)) {
            hidden.current = true;
            return;
        }
        if (hidden.current) {
            hidden.current = false;
            restored.current = true;
            root.scrollTop =
                anchor.current === 0
                    ? 0
                    : canvasOffset() + (Math.floor(anchor.current / columns) * pitch) / scale;
        }
        setVisibleHeight(root.clientHeight || height);
        if (grid) {
            const style = getComputedStyle(canvas.current);
            const card = canvas.current.querySelector<HTMLElement>(
                '.channel-main, .radio-main, .video-card',
            );
            const next = {
                columns: Math.max(1, style.gridTemplateColumns.split(/\s+/).length),
                itemHeight: card?.offsetHeight || (layout === 'radio' ? 104 : 183),
                gap: parseFloat(style.rowGap) || 0,
            };
            setGeometry((old) =>
                old.columns === next.columns &&
                old.itemHeight === next.itemHeight &&
                old.gap === next.gap
                    ? old
                    : next,
            );
        }
        readScroll.current();
    };
    const restored = useRef(false);
    const anchor = useRef(0);
    const pendingFocus = useRef<number | null>(null);
    const positions = useRef(readSession<Record<string, number>>('catalog-positions', {}));
    useLayoutEffect(() => {
        if (!active) {
            hidden.current = true;
            return;
        }
        const frame = requestAnimationFrame(() => measure.current());
        return () => cancelAnimationFrame(frame);
    }, [active]);
    useEffect(() => {
        if (!refreshInterval || !active) return;
        const timer = setInterval(() => controller.refresh(), refreshInterval);
        return () => clearInterval(timer);
    }, [controller, refreshInterval, active]);
    useEffect(() => {
        restored.current = false;
        pendingFocus.current = null;
        anchor.current = positions.current[context] || 0;
        controller.bind((o, s) => loader.current(o, s), anchor.current);
        return () => {
            positions.current = { ...positions.current, [context]: anchor.current };
            positions.current = Object.fromEntries(Object.entries(positions.current).slice(-12));
            writeSession('catalog-positions', positions.current);
            controller.close();
        };
    }, [context, revision, controller]);
    useLayoutEffect(() => {
        if (!viewport.current) return;
        const root = grid
            ? viewport.current.closest<HTMLElement>('.content-scroll')
            : viewport.current;
        scrollRoot.current = root;
        let frame = 0;
        const onScroll = () => {
            if (!frame)
                frame = requestAnimationFrame(() => {
                    frame = 0;
                    readScroll.current();
                });
        };
        const observer = new ResizeObserver(() => measure.current());
        if (root) observer.observe(root);
        if (canvas.current) observer.observe(canvas.current);
        root?.addEventListener('scroll', onScroll, { passive: true });
        measure.current();
        return () => {
            observer.disconnect();
            root?.removeEventListener('scroll', onScroll);
            cancelAnimationFrame(frame);
        };
    }, [grid, height, controller.total > 0]);
    useLayoutEffect(() => {
        measure.current();
    }, [dataRevision]);
    const lastGeometry = useRef('');
    useLayoutEffect(() => {
        const signature = `${columns}:${pitch}:${scale}`;
        const root = scrollRoot.current;
        if (
            !active ||
            hidden.current ||
            !controller.total ||
            !root ||
            (restored.current && signature === lastGeometry.current)
        )
            return;
        restored.current = true;
        lastGeometry.current = signature;
        root.scrollTop =
            grid && anchor.current === 0
                ? 0
                : canvasOffset() +
                  (Math.floor(Math.min(anchor.current, controller.total - 1) / columns) * pitch) /
                      scale;
        readScroll.current();
    }, [active, controller.total, columns, pitch, scale]);
    useEffect(() => {
        if (active && !hidden.current && controller.total && restored.current) {
            anchor.current = first;
            controller.viewport(first);
        }
    }, [active, controller, first, controller.total]);
    useLayoutEffect(() => {
        if (pendingFocus.current !== null && controller.get(pendingFocus.current)) {
            const button = viewport.current?.querySelector<HTMLElement>(
                `[data-position="${pendingFocus.current}"] button`,
            );
            if (button) {
                button.focus({ preventScroll: true });
                pendingFocus.current = null;
            }
        }
    }, [dataRevision, first, controller]);
    const moveFocus = (position: number, preserveScroll = false) => {
        const target = Math.min(Math.max(0, position), controller.total - 1);
        pendingFocus.current = target;
        const root = scrollRoot.current;
        if (root) {
            const top = Math.floor(target / columns) * pitch;
            const relativeTop =
                (grid && canvas.current
                    ? canvas.current.getBoundingClientRect().top -
                      root.getBoundingClientRect().top +
                      scroll
                    : 0) +
                top -
                scroll * scale;
            if (
                !preserveScroll &&
                (relativeTop < 0 || relativeTop + itemHeight > root.clientHeight)
            ) {
                const targetScroll = relativeTop < 0 ? top : top + itemHeight - root.clientHeight;
                root.scrollTop =
                    canvasOffset() + (targetScroll < 0 ? targetScroll : targetScroll / scale);
            }
            readScroll.current();
        }
        controller.viewport(target);
        requestAnimationFrame(() =>
            viewport.current
                ?.querySelector<HTMLElement>(`[data-position="${target}"] button`)
                ?.focus({ preventScroll: true }),
        );
    };
    return (
        <section
            className={`continuous-catalog${grid ? ' continuous-grid' : ''}`}
            aria-label={label}
        >
            <div className="library-result-count" role="status">
                {controller.loading && !controller.total
                    ? 'Caricamento…'
                    : `${controller.total.toLocaleString('it-IT')} ${itemNames[controller.total === 1 ? 0 : 1]}`}
            </div>
            {controller.error && (
                <p className="library-error" role="alert">
                    {controller.error}{' '}
                    <button className="text-button" onClick={() => controller.retry()}>
                        Riprova
                    </button>
                </p>
            )}
            {!controller.loading && !controller.total && !controller.error ? (
                empty || <p className="notice">Nessun risultato. Prova a cambiare i filtri.</p>
            ) : (
                <div
                    ref={viewport}
                    className="continuous-viewport"
                    style={grid ? undefined : { height }}
                    tabIndex={0}
                    aria-label={label}
                    aria-busy={controller.loading}
                    onKeyDown={(event) => {
                        if (
                            event.target instanceof Element &&
                            event.target.closest('input,select,textarea')
                        )
                            return;
                        const row =
                            event.target instanceof Element
                                ? event.target.closest<HTMLElement>('[data-position]')
                                : null;
                        const position = row ? Number(row.dataset.position) : first;
                        if (
                            [
                                'ArrowDown',
                                'ArrowUp',
                                ...(grid ? ['ArrowLeft', 'ArrowRight'] : []),
                                'PageDown',
                                'PageUp',
                                'Home',
                                'End',
                            ].includes(event.key)
                        ) {
                            event.preventDefault();
                            const target =
                                event.key === 'Home'
                                    ? 0
                                    : event.key === 'End'
                                      ? controller.total - 1
                                      : position +
                                        (['ArrowDown', 'ArrowRight', 'PageDown'].includes(event.key)
                                            ? 1
                                            : -1) *
                                            (event.key.startsWith('Page')
                                                ? Math.max(1, Math.floor(visibleHeight / pitch)) *
                                                  columns
                                                : event.key === 'ArrowLeft' ||
                                                    event.key === 'ArrowRight'
                                                  ? 1
                                                  : columns);
                            const sameRow =
                                Math.floor(target / columns) === Math.floor(position / columns);
                            moveFocus(
                                target,
                                sameRow && ['ArrowLeft', 'ArrowRight'].includes(event.key),
                            );
                        }
                    }}
                >
                    <div
                        ref={canvas}
                        className={`continuous-canvas${grid ? (layout === 'radio' ? ' radio-grid' : layout === 'videos' ? ' video-grid' : ' channel-grid') : ''}`}
                        role="list"
                        style={{
                            height: Math.max(physicalHeight, controller.loading ? itemHeight : 0),
                        }}
                    >
                        {Array.from({ length: end - start }, (_, i) => {
                            const position = start + i;
                            const row = controller.get(position);
                            return (
                                <div
                                    key={position}
                                    className="continuous-row"
                                    role="listitem"
                                    aria-posinset={position + 1}
                                    aria-setsize={controller.total}
                                    data-position={position}
                                    style={{
                                        height: itemHeight,
                                        ...(grid
                                            ? { gridColumn: `${(position % columns) + 1} / span 1` }
                                            : {}),
                                        transform: `translateY(${scroll - ((scroll * scale) % pitch) + (Math.floor(position / columns) - firstRow) * pitch}px)`,
                                    }}
                                >
                                    {row ? (
                                        render(row, position)
                                    ) : (
                                        <span className="library-placeholder" role="status">
                                            {controller.error
                                                ? 'Riga non disponibile'
                                                : 'Caricamento…'}
                                        </span>
                                    )}
                                </div>
                            );
                        })}
                    </div>
                </div>
            )}
        </section>
    );
}
