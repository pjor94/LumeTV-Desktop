import { app, playback } from '../app';
import { useStore } from '../store';
import { downloadPercent, updateAction, updateBusy, updateError, updates } from '../updates';
import { Icon } from './common';
import '../updates.css';

function mebibytes(bytes: number) {
    return `${(bytes / 1024 / 1024).toLocaleString('it-IT', { maximumFractionDigits: 1 })} MB`;
}
export function Updates() {
    const status = useStore(updates);
    const busy = updateBusy(status);
    const percent = downloadPercent(status.downloaded, status.total);
    const title = {
        idle: 'Controlla se c’è una nuova versione',
        checking: 'Cerco aggiornamenti…',
        up_to_date: 'Hai l’ultima versione disponibile',
        available: `È disponibile Lume TV ${status.available_version}`,
        downloading: `Scarico Lume TV ${status.available_version}`,
        ready: `Lume TV ${status.available_version} è pronta`,
        installing: 'Preparo l’installazione…',
        opened: 'Pacchetto aperto',
        error: 'Controllo non riuscito',
    }[status.phase];
    const install = async () => {
        await playback.stop();
        await updateAction('install');
    };
    return (
        <section className="settings-section update-section" aria-labelledby="updates-heading">
            <h2 id="updates-heading">Aggiornamenti dell’app</h2>
            <div className="settings-card">
                <div className="settings-row">
                    <span className="settings-icon">
                        <Icon name="refresh" />
                    </span>
                    <div className="settings-label">
                        <strong>{title}</strong>
                        <p>Versione installata: {status.current_version || app.get().version}.</p>
                    </div>
                    <button
                        className="text-button"
                        disabled={busy}
                        onClick={() => void updateAction('check', { manual: true })}
                    >
                        Controlla
                    </button>
                </div>
                {status.available_version && (
                    <div className="update-detail">
                        {status.notes && <p className="update-notes">{status.notes}</p>}
                        {status.phase === 'downloading' && (
                            <div className="update-download">
                                <div className="update-transfer">
                                    <span>
                                        {mebibytes(status.downloaded)} di {mebibytes(status.total)}
                                    </span>
                                    <strong>{percent}%</strong>
                                </div>
                                <progress
                                    max="100"
                                    value={percent}
                                    aria-label="Download aggiornamento"
                                />
                                <p>Il pacchetto verrà verificato prima dell’installazione.</p>
                            </div>
                        )}
                        {status.phase === 'ready' && (
                            <p>
                                {status.install_mode === 'replace'
                                    ? 'L’app si chiuderà e si riaprirà aggiornata. Preferiti e impostazioni restano salvati.'
                                    : status.install_mode === 'system'
                                      ? 'Lume si chiuderà. Completa l’installazione con il gestore dei pacchetti, poi riapri l’app.'
                                      : 'Apri il pacchetto e sostituisci la copia dell’app. Preferiti e impostazioni restano salvati.'}
                            </p>
                        )}
                        {status.phase === 'opened' && (
                            <p>
                                Completa l’installazione nella finestra aperta, poi riapri Lume TV.
                            </p>
                        )}
                        <div className="update-actions">
                            <button
                                className="text-button"
                                disabled={
                                    status.phase === 'checking' || status.phase === 'installing'
                                }
                                onClick={() => {
                                    if (status.phase === 'downloading') void updateAction('cancel');
                                    else if (status.phase === 'ready' || status.phase === 'opened')
                                        void install();
                                    else void updateAction('download');
                                }}
                            >
                                {status.phase === 'downloading'
                                    ? 'Annulla download'
                                    : status.phase === 'ready' || status.phase === 'opened'
                                      ? status.install_mode === 'replace'
                                          ? 'Installa e riavvia'
                                          : status.install_mode === 'system'
                                            ? 'Apri installazione'
                                            : 'Apri pacchetto'
                                      : `Scarica aggiornamento · ${mebibytes(status.total)}`}
                            </button>
                        </div>
                    </div>
                )}
                {status.error && (
                    <p className="update-message" role="status">
                        {updateError(status.error)}
                    </p>
                )}
                {status.last_install_failed && (
                    <p className="update-message">
                        L’ultimo aggiornamento non è riuscito. Puoi scaricare nuovamente il
                        pacchetto; la versione precedente è stata conservata.
                    </p>
                )}
                {(['automatic', 'preview'] as const).map((key) => (
                    <div className="settings-row" key={key}>
                        <span className="settings-icon">
                            <Icon name={key === 'automatic' ? 'clock' : 'tv'} />
                        </span>
                        <div className="settings-label">
                            <strong>
                                {key === 'automatic'
                                    ? 'Controlla automaticamente'
                                    : 'Versioni di prova'}
                            </strong>
                            <p>
                                {key === 'automatic'
                                    ? 'Lume controlla ogni mezz’ora mentre è aperta. Decidi tu quando scaricare e installare.'
                                    : 'Ricevi anche le nuove versioni sperimentali del desktop.'}
                            </p>
                        </div>
                        <button
                            className="toggle"
                            role="switch"
                            aria-label={
                                key === 'automatic'
                                    ? 'Controlla automaticamente gli aggiornamenti'
                                    : 'Includi versioni di prova'
                            }
                            aria-checked={status.settings[key]}
                            disabled={busy}
                            onClick={() =>
                                void updateAction('settings', { [key]: !status.settings[key] })
                            }
                        />
                    </div>
                ))}
            </div>
        </section>
    );
}
