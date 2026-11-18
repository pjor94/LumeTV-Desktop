import { useEffect, useState } from 'react';
import { app, closeDialog, playback, savePreferences } from '../app';
import { catalogChannel, errorMessage } from '../personal';
import type { Channel } from '../types';
import { useStore } from '../store';
import { Icon } from './common';
export function ChannelCustomization({ id }: { id: string }) {
    const [channel, setChannel] = useState<Channel | undefined>(() =>
        app.get().channels.find((c) => c.id === id),
    );
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState('');
    useEffect(() => {
        if (channel) return;
        let cancelled = false;
        void catalogChannel(id)
            .then((c) => {
                if (!cancelled) setChannel(c || undefined);
            })
            .catch((e) => {
                if (!cancelled) setError(errorMessage(e));
            });
        return () => {
            cancelled = true;
        };
    }, [id]);
    const change = async (patch: object, message: string, hide = false) => {
        setBusy(true);
        setError('');
        try {
            if (!(await savePreferences(patch, message))) {
                setError('Non riesco a salvare la modifica. Riprova.');
                return;
            }
            if (hide && playback.store.get().current === id) await playback.stop();
            closeDialog();
        } catch (e) {
            setError(errorMessage(e));
        } finally {
            setBusy(false);
        }
    };
    return (
        <>
            <div className="dialog-top">
                <Icon name="settings" />
                <button className="icon-button" aria-label="Chiudi" onClick={closeDialog}>
                    <Icon name="close" />
                </button>
            </div>
            <h2 id="dialog-title">{channel?.name || 'Personalizza canale'}</h2>
            <p>
                La scelta vale per questa lista. Puoi ripristinare i canali nascosti dalle
                impostazioni.
            </p>
            <div className="dialog-actions">
                <button
                    className="button secondary"
                    disabled={!channel || busy}
                    onClick={() =>
                        void change({ channel_first: id }, 'Canale spostato all’inizio.')
                    }
                >
                    Sposta all’inizio
                </button>
                <button
                    className="button secondary"
                    disabled={!channel || busy}
                    onClick={() => void change({ hide_channel: id }, 'Canale nascosto.', true)}
                >
                    Nascondi canale
                </button>
            </div>
            {error && <p role="alert">{error}</p>}
        </>
    );
}
export function HiddenChannelsSettings() {
    const prefs = useStore(app).preferences;
    const hidden = prefs.channel_layout?.hidden || [];
    const order = prefs.channel_layout?.order || [];
    const [page, setPage] = useState(0);
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState('');
    if (!hidden.length && !order.length) return null;
    const offset = Math.min(page * 20, Math.max(0, Math.ceil(hidden.length / 20) - 1) * 20);
    const change = async (patch: object) => {
        setBusy(true);
        setError('');
        try {
            if (!(await savePreferences(patch)))
                setError('Non riesco a salvare la modifica. Riprova.');
        } catch (e) {
            setError(errorMessage(e));
        } finally {
            setBusy(false);
        }
    };
    return (
        <section aria-label="Personalizzazione dei canali">
            <h2>I tuoi canali</h2>
            <div className="settings-card companion-content">
                {!!order.length && (
                    <div className="video-actions">
                        <p>
                            Ordine personalizzato per {order.length.toLocaleString('it-IT')} canali.
                        </p>
                        <button
                            className="text-button"
                            disabled={busy}
                            onClick={() => void change({ reset_channel_order: true })}
                        >
                            Ripristina ordine della lista
                        </button>
                    </div>
                )}
                {!!hidden.length && (
                    <>
                        <h3>Canali nascosti ({hidden.length.toLocaleString('it-IT')})</h3>
                        {hidden.slice(offset, offset + 20).map((c) => (
                            <div className="companion-device" key={c.id}>
                                <span>{c.name}</span>
                                <button
                                    className="text-button"
                                    disabled={busy}
                                    onClick={() => void change({ restore_hidden: c.id })}
                                >
                                    Ripristina {c.name}
                                </button>
                            </div>
                        ))}
                        {hidden.length > 20 && (
                            <div className="video-actions">
                                <button
                                    className="text-button"
                                    disabled={offset === 0}
                                    onClick={() => setPage(Math.max(0, offset / 20 - 1))}
                                >
                                    Precedenti
                                </button>
                                <span>
                                    {offset + 1}–{Math.min(offset + 20, hidden.length)}
                                </span>
                                <button
                                    className="text-button"
                                    disabled={offset + 20 >= hidden.length}
                                    onClick={() => setPage(offset / 20 + 1)}
                                >
                                    Successivi
                                </button>
                            </div>
                        )}
                    </>
                )}
                {error && <p role="alert">{error}</p>}
            </div>
        </section>
    );
}
