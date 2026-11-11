import { useLayoutEffect, useRef } from 'react';
import { app, closeDialog, navigate, play, toggleReminder } from '../app';
import { date, range, schedule, time } from '../domain';
import { motion } from '../motion';
import { Icon, StationLogo } from './common';

const Close = () => (
    <button
        className="icon-button"
        data-action="close-dialog"
        aria-label="Chiudi"
        onClick={closeDialog}
    >
        <Icon name="close" />
    </button>
);
function ProgrammeDetails({ programmeKey }: { programmeKey: string }) {
    const state = app.get();
    const [id, start] = programmeKey.split('~');
    const channel = state.channels.find((c) => c.id === id);
    const p = schedule(state.guide.programmes[id]).find((p) => String(p.start) === start);
    if (!channel || !p)
        return (
            <>
                <Close />
                <h2 id="dialog-title">Programma non più disponibile.</h2>
                <p className="programme-description">
                    La guida è stata aggiornata. Scegli un altro programma.
                </p>
            </>
        );
    const reminder = state.preferences.reminders.some(
        (r) => r.channel_id === id && r.start === p.start,
    );
    const future = p.start > state.now;
    return (
        <>
            <div className="dialog-top">
                <StationLogo channel={channel} />
                <span>
                    <strong>{channel.name}</strong>
                    <small className="programme-meta">{date(p.start)}</small>
                </span>
                <Close />
            </div>
            <div className="eyebrow">
                {future ? 'A SEGUIRE' : p.end > state.now ? 'IN ONDA ADESSO' : 'GIÀ TRASMESSO'}
            </div>
            <h2 id="dialog-title">{p.title}</h2>
            <p className="programme-time">{range(p)}</p>
            <p className="programme-description">
                {p.description ||
                    'La descrizione di questo programma non è disponibile nella guida.'}
            </p>
            <div className="dialog-actions">
                <button className="button primary" data-play={id} onClick={() => play(id)}>
                    <Icon name="play" />
                    Guarda {future || p.end <= state.now ? 'il canale' : 'ora'}
                </button>
                {future && (
                    <button
                        className="button secondary"
                        data-remind={programmeKey}
                        aria-pressed={reminder}
                        onClick={() => void toggleReminder(programmeKey)}
                    >
                        <Icon name={reminder ? 'check' : 'bell'} />
                        {reminder ? 'Promemoria attivo' : 'Ricordamelo'}
                    </button>
                )}
            </div>
            {future && (
                <p className="settings-note">
                    Ti avviseremo poco prima dell’inizio, mentre Lume è aperta.
                </p>
            )}
        </>
    );
}
function Reminders() {
    const state = app.get();
    const list = state.preferences.reminders
        .filter((r) => r.start > state.now)
        .sort((a, b) => a.start - b.start);
    return (
        <>
            <div className="dialog-top">
                <div
                    className="empty-symbol"
                    style={{ margin: 0, width: 40, height: 40, borderRadius: 12 }}
                >
                    <Icon name="bell" />
                </div>
                <Close />
            </div>
            <h2 id="dialog-title">Un momento da ricordare.</h2>
            <p className="programme-meta">Gli avvisi arrivano mentre Lume è aperta.</p>
            {list.length ? (
                <div className="reminder-list">
                    {list.map((r) => (
                        <div key={`${r.channel_id}~${r.start}`} className="reminder-item">
                            <div>
                                <strong>{r.title}</strong>
                                <small>
                                    {state.channels.find((c) => c.id === r.channel_id)?.name || ''}{' '}
                                    · {date(r.start)}, {time(r.start)}
                                </small>
                            </div>
                            <button
                                className="icon-button"
                                data-remove-reminder={`${r.channel_id}~${r.start}`}
                                aria-label={`Rimuovi promemoria per ${r.title}`}
                                onClick={() => void toggleReminder(`${r.channel_id}~${r.start}`)}
                            >
                                <Icon name="close" />
                            </button>
                        </div>
                    ))}
                </div>
            ) : (
                <>
                    <p className="programme-description">
                        Hai visto qualcosa di interessante nella guida? Apri il programma e premi
                        “Ricordamelo”. Lo ritroverai qui.
                    </p>
                    <button
                        className="button secondary"
                        data-route="guide"
                        onClick={() => navigate('guide')}
                    >
                        Esplora la guida
                    </button>
                </>
            )}
        </>
    );
}
function Shortcuts() {
    const modifier = /Mac/i.test(navigator.platform) ? '⌘' : 'Ctrl';
    return (
        <>
            <div className="dialog-top">
                <span className="eyebrow" style={{ margin: 0 }}>
                    TUTTO A PORTATA DI MANO
                </span>
                <Close />
            </div>
            <h2 id="dialog-title">Il piacere di fare prima.</h2>
            <div className="shortcut-list">
                {[
                    ['Cerca un canale o programma', `${modifier} K`],
                    ['Canali · Cosa guardare · Guida · Preferiti', `${modifier} 1 / 2 / 3 / 4`],
                    ['Play / Pausa', 'Spazio'],
                    ['Attiva / Disattiva audio', 'M'],
                    ['Schermo intero', 'F'],
                    ['Canale precedente / successivo', '↑ / ↓'],
                    ['Torna al catalogo durante la diretta', 'Esc'],
                ].map(([label, key]) => (
                    <div key={label}>
                        <span>{label}</span>
                        <kbd>{key}</kbd>
                    </div>
                ))}
            </div>
        </>
    );
}
export function Dialogs() {
    const state = app.get();
    const ref = useRef<HTMLDialogElement>(null);
    useLayoutEffect(() => {
        const dialog = ref.current!;
        if (state.dialogOpen) motion.openDialog(dialog);
        else motion.closeDialog(dialog);
    }, [state.dialogOpen, state.dialog]);
    return (
        <dialog
            ref={ref}
            id="details-dialog"
            aria-labelledby="dialog-title"
            onCancel={(event) => {
                event.preventDefault();
                closeDialog();
            }}
            onClick={(event) => {
                if (event.target === event.currentTarget) closeDialog();
            }}
        >
            <div className="dialog-content" id="dialog-content">
                {state.dialog?.kind === 'programme' ? (
                    <ProgrammeDetails programmeKey={state.dialog.key} />
                ) : state.dialog?.kind === 'reminders' ? (
                    <Reminders />
                ) : state.dialog?.kind === 'shortcuts' ? (
                    <Shortcuts />
                ) : null}
            </div>
        </dialog>
    );
}
