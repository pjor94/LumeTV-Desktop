import type { ReactNode } from 'react';
import { app, refreshCatalog, refreshGuide, savePreferences, showShortcuts } from '../app';
import { date, time } from '../domain';
import { Heading, Icon } from '../components/common';
import type { Preferences, Quality } from '../types';
import { Updates } from '../components/Updates';

function Row({
    symbol,
    title,
    description,
    children,
}: {
    symbol: string;
    title: string;
    description: string;
    children: ReactNode;
}) {
    return (
        <div className="settings-row">
            <span className="settings-icon">
                <Icon name={symbol} />
            </span>
            <div className="settings-label">
                <strong>{title}</strong>
                <p>{description}</p>
            </div>
            {children}
        </div>
    );
}
function Toggle({ name, label }: { name: 'muted' | 'zap_favorites_only'; label: string }) {
    const value = app.get().preferences[name];
    return (
        <button
            className="toggle"
            role="switch"
            aria-label={label}
            aria-checked={value}
            data-setting-toggle={name}
            onClick={() => void savePreferences({ [name]: !value })}
        />
    );
}
export function QualityOptions({ settings = false }: { settings?: boolean }) {
    return (
        <>
            <option value="auto">Automatica</option>
            {['1080', '720', '480'].map((q) => (
                <option key={q} value={q}>
                    {settings ? 'Fino a ' : ''}
                    {q}p
                </option>
            ))}
        </>
    );
}
export function Settings() {
    const state = app.get();
    const p = state.preferences;
    return (
        <>
            <Heading
                title="Lume, a modo tuo."
                subtitle="I piccoli dettagli che rendono la TV ancora più tua."
            />
            <div className="settings-layout">
                <section className="settings-section">
                    <h2>Il tuo spazio</h2>
                    <div className="settings-card">
                        <Row
                            symbol="moon"
                            title="Aspetto"
                            description="Scegli la luce giusta per il tuo momento."
                        >
                            <div className="segmented" aria-label="Aspetto">
                                {(
                                    [
                                        ['dark', 'Scuro'],
                                        ['light', 'Chiaro'],
                                        ['system', 'Sistema'],
                                    ] as [Preferences['theme'], string][]
                                ).map(([id, label]) => (
                                    <button
                                        key={id}
                                        data-theme={id}
                                        aria-pressed={p.theme === id}
                                        onClick={() => void savePreferences({ theme: id })}
                                    >
                                        {label}
                                    </button>
                                ))}
                            </div>
                        </Row>
                        <Row
                            symbol="star"
                            title="Cambia canale tra i preferiti"
                            description="Usa i tasti ↑ e ↓ per passare solo tra i canali che ami."
                        >
                            <Toggle
                                name="zap_favorites_only"
                                label="Cambia canale solo tra i preferiti"
                            />
                        </Row>
                    </div>
                </section>
                <section className="settings-section">
                    <h2>La tua visione</h2>
                    <div className="settings-card">
                        <Row
                            symbol="tv"
                            title="Qualità massima"
                            description="Lume adatta la qualità alla connessione, fino al limite scelto."
                        >
                            <select
                                id="settings-quality"
                                aria-label="Qualità massima"
                                value={p.quality}
                                onChange={(event) =>
                                    void savePreferences({ quality: event.target.value as Quality })
                                }
                            >
                                <QualityOptions settings />
                            </select>
                        </Row>
                        <Row
                            symbol="volume"
                            title="Avvia con audio disattivato"
                            description="Il volume scelto viene ricordato per la prossima diretta."
                        >
                            <Toggle name="muted" label="Avvia con audio disattivato" />
                        </Row>
                    </div>
                </section>
                <section className="settings-section">
                    <h2>Sempre al passo</h2>
                    <div className="settings-card">
                        <Row
                            symbol="grid"
                            title="Catalogo canali"
                            description={
                                state.catalog.error ||
                                (state.catalog.loading
                                    ? 'Stiamo aggiornando la lista…'
                                    : `${state.channels.length} canali · ${state.catalog.updated_at ? 'aggiornato ' + date(state.catalog.updated_at) : 'catalogo incluso nell’app'}. Aggiornamento automatico ogni giorno.`)
                            }
                        >
                            <button
                                className="text-button"
                                data-action="refresh-catalog"
                                aria-label="Aggiorna catalogo canali"
                                disabled={
                                    state.catalog.loading || state.catalog.next_retry_at > state.now
                                }
                                onClick={() => void refreshCatalog(true)}
                            >
                                <Icon name="refresh" />
                                Aggiorna
                            </button>
                        </Row>
                        <Row
                            symbol="guide"
                            title="Guida TV"
                            description={
                                (state.guide.next_retry_at || 0) > state.now
                                    ? `Il servizio richiede una pausa. Prossimo tentativo dalle ${time(state.guide.next_retry_at!)}.`
                                    : state.guide.loading
                                      ? 'Stiamo aggiornando i programmi…'
                                      : state.guide.updated_at
                                        ? `Ultimo aggiornamento: ${date(state.guide.updated_at)}, ${time(state.guide.updated_at)}.`
                                        : 'La guida viene aggiornata automaticamente.'
                            }
                        >
                            <button
                                className="text-button"
                                data-action="refresh-guide"
                                disabled={
                                    state.guide.loading ||
                                    (state.guide.next_retry_at || 0) > state.now
                                }
                                onClick={() => void refreshGuide(true)}
                            >
                                <Icon name="refresh" />
                                Aggiorna
                            </button>
                        </Row>
                        <Row
                            symbol="clock"
                            title="Canali visti di recente"
                            description="La tua cronologia rimane su questo dispositivo."
                        >
                            <button
                                className="text-button"
                                data-action="clear-recent"
                                disabled={!p.recent.length}
                                onClick={() =>
                                    void savePreferences(
                                        { clear_recent: true },
                                        'Cronologia svuotata.',
                                    )
                                }
                            >
                                Svuota cronologia
                            </button>
                        </Row>
                        <Row
                            symbol="keyboard"
                            title="Scorciatoie da tastiera"
                            description="Un modo ancora più veloce per sentirti a casa."
                        >
                            <button
                                className="text-button"
                                data-action="shortcuts"
                                onClick={showShortcuts}
                            >
                                Scoprile <Icon name="arrow" />
                            </button>
                        </Row>
                    </div>
                    <p className="settings-note">
                        I promemoria ti avvisano mentre Lume è aperta. Canali e programmi possono
                        cambiare in base alla disponibilità delle emittenti.
                    </p>
                </section>
                <Updates />
                <div className="about-lume">
                    <img src="/assets/symbol.svg" alt="" />
                    <div>
                        <strong>Lume TV</strong>
                        <p>La TV, fatta semplice. · Versione {state.version}</p>
                    </div>
                </div>
            </div>
        </>
    );
}
