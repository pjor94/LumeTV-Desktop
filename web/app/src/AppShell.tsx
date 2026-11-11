import { useEffect, useLayoutEffect, useRef } from 'react';
import {
    app,
    dismissToast,
    navigate,
    playback,
    setQuery,
    showReminders,
    showShortcuts,
    takeDockTransition,
    zap,
    toast,
} from './app';
import { time } from './domain';
import { useStore, useStoreValue } from './store';
import { motion } from './motion';
import { Dialogs } from './components/Dialogs';
import { IconDefinitions } from './components/IconDefinitions';
import { EmptyState, Equalizer, Icon, StationLogo } from './components/common';
import { Player, fullscreen, toggleMute } from './components/Player';
import { WatchBrowser, WatchDetails } from './components/WatchDetails';
import { Channels } from './pages/Channels';
import { Discovery } from './pages/Discovery';
import { GuidePage } from './pages/Guide';
import { Search } from './pages/Search';
import { Settings } from './pages/Settings';
import type { Route } from './types';
import { connectUpdates, updates } from './updates';

const routes: [Route, string, string][] = [
    ['channels', 'Canali', 'grid'],
    ['discover', 'Cosa guardare', 'compass'],
    ['guide', 'Guida TV', 'guide'],
    ['favorites', 'Preferiti', 'star'],
];
export function App() {
    const state = useStore(app);
    const updateStatus = useStore(updates);
    const notifiedUpdate = useRef('');
    const current = useStoreValue(playback.store, (state) => state.current);
    useEffect(connectUpdates, []);
    useEffect(() => {
        const version = updateStatus.available_version;
        if (version && version !== notifiedUpdate.current && !current && !state.dialogOpen) {
            notifiedUpdate.current = version;
            toast(`È disponibile Lume TV ${version}.`, {
                label: 'Aggiornamenti',
                run: () => navigate('settings'),
            });
        }
    }, [updateStatus.available_version, current, state.dialogOpen]);
    const channel = playback.store.get().channel;
    const search = useRef<HTMLInputElement>(null);
    const scroll = useRef<HTMLDivElement>(null);
    const effectivePage = state.query ? 'search' : state.page;
    useLayoutEffect(() => {
        document.body.dataset.page = effectivePage;
        const dock = document.getElementById('player-dock');
        const before = takeDockTransition();
        if (dock && before) motion.moveDock(dock, before);
        if (state.page === 'watch' && channel) document.title = `${channel.name} · Lume TV`;
        else document.title = 'Lume TV';
    }, [effectivePage, channel]);
    useLayoutEffect(() => {
        if (scroll.current) scroll.current.scrollTop = 0;
    }, [state.scrollRevision]);
    useEffect(() => {
        const query = matchMedia('(prefers-color-scheme: dark)');
        const apply = () => {
            document.documentElement.dataset.theme =
                state.preferences.theme === 'system'
                    ? query.matches
                        ? 'dark'
                        : 'light'
                    : state.preferences.theme;
        };
        apply();
        query.addEventListener('change', apply);
        return () => query.removeEventListener('change', apply);
    }, [state.preferences.theme]);
    useEffect(() => {
        const keyboard = (event: KeyboardEvent) => {
            const modifier = event.metaKey || event.ctrlKey;
            if (modifier && event.key.toLowerCase() === 'k') {
                event.preventDefault();
                search.current?.focus();
                search.current?.select();
                return;
            }
            if (modifier && ['1', '2', '3', '4'].includes(event.key)) {
                event.preventDefault();
                navigate(routes[Number(event.key) - 1][0]);
                return;
            }
            const state = app.get();
            if (state.dialogOpen) return;
            if (event.key === 'Escape') {
                if (document.fullscreenElement) {
                    event.preventDefault();
                    void document.exitFullscreen().catch(() => {});
                } else if (state.query) {
                    setQuery('');
                    search.current?.blur();
                } else if (state.page === 'watch')
                    navigate(state.previousPage === 'watch' ? 'channels' : state.previousPage);
                return;
            }
            const target = event.target instanceof Element ? event.target : null;
            if (
                target?.closest('input,select,textarea') ||
                modifier ||
                state.page !== 'watch' ||
                (event.key === ' ' && target?.closest('button'))
            )
                return;
            if (event.key === ' ') {
                event.preventDefault();
                playback.togglePause();
            }
            if (event.key.toLowerCase() === 'm') toggleMute();
            if (event.key.toLowerCase() === 'f') void fullscreen();
            if (event.key === 'ArrowDown' || event.key === 'PageDown') {
                event.preventDefault();
                zap(1);
            }
            if (event.key === 'ArrowUp' || event.key === 'PageUp') {
                event.preventDefault();
                zap(-1);
            }
        };
        document.addEventListener('keydown', keyboard);
        return () => document.removeEventListener('keydown', keyboard);
    }, []);
    const page = state.page === 'watch' ? state.previousPage : state.page;
    const reminders = state.preferences.reminders.filter((r) => r.start > state.now).length;
    return (
        <>
            <IconDefinitions />
            <a className="skip-link" href="#page-content">
                Vai ai contenuti
            </a>
            <aside className="sidebar" aria-label="Navigazione principale">
                <a
                    className="brand"
                    href="#channels"
                    aria-label="Lume TV, canali"
                    onClick={(event) => {
                        event.preventDefault();
                        navigate('channels');
                    }}
                >
                    <img className="brand-light" src="/assets/logo-light.svg" alt="Lume TV" />
                    <img className="brand-dark" src="/assets/logo-dark.svg" alt="Lume TV" />
                    <img className="brand-symbol" src="/assets/symbol.svg" alt="" />
                </a>
                <span className="sidebar-label">LA TUA TELEVISIONE</span>
                <nav className="main-navigation">
                    {routes.map(([route, name, icon]) => (
                        <button
                            key={route}
                            className={`nav-item ${state.page === route && !state.query ? 'active' : ''}`}
                            data-route={route}
                            aria-label={name}
                            title={name}
                            aria-current={state.page === route && !state.query ? 'page' : undefined}
                            onClick={() => navigate(route)}
                        >
                            <Icon name={icon} />
                            <span>{name}</span>
                            {route === 'favorites' && (
                                <span
                                    className="nav-count"
                                    id="favorite-count"
                                    hidden={!state.preferences.favorites.length}
                                >
                                    {state.preferences.favorites.length}
                                </span>
                            )}
                        </button>
                    ))}
                </nav>
                <div className="sidebar-now" id="sidebar-now" hidden={!channel}>
                    {channel && (
                        <button data-route="watch" onClick={() => navigate('watch')}>
                            <StationLogo channel={channel} />
                            <span>
                                <strong>{channel.name}</strong>
                                <small>Stai guardando</small>
                            </span>
                            <Equalizer />
                        </button>
                    )}
                </div>
                <div className="sidebar-bottom">
                    <button
                        className="nav-item"
                        id="reminders-button"
                        aria-label="Promemoria"
                        title="Promemoria"
                        onClick={showReminders}
                    >
                        <Icon name="bell" />
                        <span>Promemoria</span>
                        <span className="nav-count" id="reminder-count" hidden={!reminders}>
                            {reminders}
                        </span>
                    </button>
                    <button
                        className={`nav-item ${state.page === 'settings' && !state.query ? 'active' : ''}`}
                        data-route="settings"
                        aria-label="Impostazioni"
                        title="Impostazioni"
                        aria-current={
                            state.page === 'settings' && !state.query ? 'page' : undefined
                        }
                        onClick={() => navigate('settings')}
                    >
                        <Icon name="settings" />
                        <span>Impostazioni</span>
                        {updateStatus.available_version && (
                            <span
                                className="update-indicator"
                                role="img"
                                aria-label="Aggiornamento disponibile"
                            />
                        )}
                    </button>
                    <div className="sidebar-signature">
                        <span className="lume-dot" />
                        La TV, fatta semplice.
                    </div>
                </div>
            </aside>
            <main className="workspace">
                <header className="toolbar">
                    <div className="toolbar-location">
                        <span className="live-dot" />
                        <span>Un mondo da guardare</span>
                    </div>
                    <label className="search-box">
                        <Icon name="search" />
                        <span className="sr-only">Cerca canale o programma</span>
                        <input
                            ref={search}
                            id="search"
                            type="search"
                            autoComplete="off"
                            placeholder="Cerca canale o programma…"
                            maxLength={120}
                            value={state.query}
                            onChange={(event) => setQuery(event.target.value)}
                        />
                        <kbd id="search-shortcut">
                            {/Mac/i.test(navigator.platform) ? '⌘ K' : 'Ctrl K'}
                        </kbd>
                    </label>
                    <time id="clock" className="clock" dateTime={new Date(state.now).toISOString()}>
                        {time(state.now)}
                    </time>
                </header>
                <div ref={scroll} className="content-scroll" id="content-scroll">
                    <div
                        id="page-content"
                        tabIndex={-1}
                        hidden={state.page === 'watch' && !state.query}
                    >
                        {state.failed ? (
                            <EmptyState
                                symbol="wifi"
                                title="Ritroviamo la connessione con Lume."
                                description="Chiudi e riapri l’app per ripartire con una nuova sessione. I tuoi preferiti restano al sicuro."
                            />
                        ) : !state.ready ? (
                            <div className="initial-loading">
                                <img src="/assets/symbol.svg" alt="" />
                                <p>Metti comodo il tuo mondo.</p>
                                <span>Caricamento dei canali…</span>
                            </div>
                        ) : state.query ? (
                            <Search />
                        ) : page === 'discover' ? (
                            <Discovery />
                        ) : page === 'guide' ? (
                            <GuidePage />
                        ) : page === 'settings' ? (
                            <Settings />
                        ) : (
                            <Channels favoritesOnly={page === 'favorites'} />
                        )}
                    </div>
                    <section id="watch-view" aria-label="Diretta" hidden={!current}>
                        <Player />
                        <div className="watch-details" id="watch-details">
                            <WatchDetails />
                        </div>
                        <aside
                            className="watch-browser"
                            id="watch-browser"
                            aria-label="Cambia canale"
                        >
                            <WatchBrowser />
                        </aside>
                    </section>
                    <footer className="content-footer">
                        <span id="catalog-status">
                            {state.guide.loading
                                ? 'La tua guida sta arrivando…'
                                : state.guide.error
                                  ? 'Canali disponibili · Guida da aggiornare'
                                  : state.guide.updated_at
                                    ? `Guida aggiornata alle ${time(state.guide.updated_at)}`
                                    : 'I tuoi canali, in un solo posto.'}
                        </span>
                        <button
                            className="text-button"
                            id="shortcuts-button"
                            onClick={showShortcuts}
                        >
                            <Icon name="keyboard" />
                            Scorciatoie da tastiera
                        </button>
                    </footer>
                </div>
            </main>
            <div id="toast-region" className="toast-region" role="status" aria-live="polite">
                {state.toasts.map((t) => (
                    <div
                        key={t.id}
                        className={`toast ${t.dismissing ? 'removing' : ''}`}
                        data-instant={!motion.canMove() ? '' : undefined}
                    >
                        <Icon name={t.action ? 'bell' : 'check'} />
                        <span>{t.message}</span>
                        {t.action && (
                            <button
                                disabled={t.dismissing}
                                onClick={() => {
                                    t.action!.run();
                                    dismissToast(t.id);
                                }}
                            >
                                {t.action.label}
                            </button>
                        )}
                    </div>
                ))}
            </div>
            <Dialogs />
        </>
    );
}
