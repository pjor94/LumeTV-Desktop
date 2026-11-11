import { useEffect } from 'react';
import { app, startup } from '../app';
import { useStore, useStoreValue } from '../store';
import { App } from '../AppShell';

function StartupScreen() {
    const state = useStore(startup.store);
    const failed = useStoreValue(app, (value) => value.failed);
    const connected = useStoreValue(app, (value) => value.ready);
    const theme = useStoreValue(app, (value) => value.preferences.theme);
    const busy = !failed && ['checking', 'preparing'].includes(state.phase);
    useEffect(() => {
        document.body.dataset.page = 'setup';
        const media = matchMedia('(prefers-color-scheme: dark)');
        const apply = () => {
            document.documentElement.dataset.theme =
                theme === 'system' ? (media.matches ? 'dark' : 'light') : theme;
        };
        apply();
        media.addEventListener('change', apply);
        return () => media.removeEventListener('change', apply);
    }, [theme]);
    return (
        <main className="startup-screen" aria-labelledby="startup-title">
            <div className="startup-content">
                <div className="startup-brand">
                    <img className="brand-light" src="/assets/logo-light.svg" alt="Lume TV" />
                    <img className="brand-dark" src="/assets/logo-dark.svg" alt="Lume TV" />
                </div>
                <h1 id="startup-title">Prepariamo Lume.</h1>
                <p className="startup-introduction">
                    Al primo avvio Lume prepara Widevine, il componente necessario per guardare i
                    canali protetti.
                </p>
                <div className="startup-status" role="status" aria-live="polite" aria-atomic="true">
                    <h2>
                        {failed
                            ? 'La sessione locale non è disponibile.'
                            : !connected
                              ? 'Avvio di Lume…'
                              : state.title}
                    </h2>
                    <p>
                        {failed
                            ? 'Chiudi e riapri Lume per avviare una nuova sessione. I tuoi preferiti restano salvati.'
                            : !connected
                              ? 'Ancora un istante.'
                              : state.detail}
                    </p>
                </div>
                {busy && (
                    <div
                        className="widevine-progress startup-progress"
                        role="progressbar"
                        aria-label="Preparazione di Lume"
                        aria-valuetext={state.title}
                    >
                        <span />
                    </div>
                )}
                {state.phase === 'preparing' && (
                    <p className="startup-elapsed" aria-live="off">
                        Tempo trascorso: {Math.floor(state.elapsed / 60)}:
                        {String(Math.floor(state.elapsed % 60)).padStart(2, '0')}
                    </p>
                )}
                {!failed && state.phase === 'error' && (
                    <button
                        className="button primary"
                        onClick={() => void startup.start()}
                        autoFocus
                    >
                        Riprova
                    </button>
                )}
                {state.phase === 'restart' ? (
                    <p className="startup-note">
                        Chiudi la finestra, poi riapri Lume dall’icona dell’app.
                    </p>
                ) : busy ? (
                    <p className="startup-note">
                        Il download avviene dai servizi Google e può richiedere qualche minuto.
                        Lascia Lume aperta fino al completamento.
                    </p>
                ) : null}
            </div>
        </main>
    );
}

export function DesktopApp() {
    const ready = useStoreValue(app, (value) => value.ready);
    const failed = useStoreValue(app, (value) => value.failed);
    const phase = useStoreValue(startup.store, (value) => value.phase);
    return ready && !failed && phase === 'ready' ? <App /> : <StartupScreen />;
}
