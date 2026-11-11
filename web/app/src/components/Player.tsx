import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { app, navigate, playback, savePreferences, toast } from '../app';
import { useStore } from '../store';
import { motion } from '../motion';
import { QualityOptions } from '../pages/Settings';
import { Icon } from './common';
import type { Quality } from '../types';

export async function fullscreen() {
    try {
        if (document.fullscreenElement) await document.exitFullscreen();
        else await document.getElementById('player-dock')?.requestFullscreen();
    } catch {
        toast('Lo schermo intero non è disponibile in questa finestra.');
    }
}
export function toggleMute() {
    const state = playback.store.get();
    playback.volume(state.volume, !state.muted);
    void savePreferences({ muted: !state.muted });
}
export function Player() {
    const state = useStore(playback.store);
    const preferences = app.get().preferences;
    const video = useRef<HTMLVideoElement>(null);
    const dock = useRef<HTMLDivElement>(null);
    const options = useRef<HTMLDivElement>(null);
    const [expanded, setExpanded] = useState(false);
    const [controlsHidden, setControlsHidden] = useState(false);
    const hideTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
    const volumeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
    const pendingVolume = useRef<{ volume: number; muted: boolean } | null>(null);
    useEffect(() => playback.attach(video.current!), []);
    useLayoutEffect(() => {
        motion.setOptions(options.current!, expanded);
    }, [expanded]);
    useEffect(() => {
        setControlsHidden(false);
    }, [state.phase]);
    useEffect(() => {
        const unsubscribe = app.subscribe(() => {
            if (app.get().page !== 'watch' || app.get().query) setExpanded(false);
        });
        return () => {
            unsubscribe();
            clearTimeout(hideTimer.current);
            clearTimeout(volumeTimer.current);
            if (pendingVolume.current) void savePreferences(pendingVolume.current);
            motion.stopDock(dock.current!);
        };
    }, []);
    const languageName = (language: string) => {
        try {
            return new Intl.DisplayNames(['it'], { type: 'language' }).of(language) || language;
        } catch {
            return language;
        }
    };
    const overlay = !['playing', 'paused'].includes(state.phase);
    return (
        <div
            ref={dock}
            id="player-dock"
            className={`player-dock ${state.hasPicture ? 'has-picture' : ''} ${controlsHidden && !overlay ? 'controls-hidden' : ''}`}
            tabIndex={0}
            aria-label="Player video"
            onMouseMove={() => {
                setControlsHidden(false);
                clearTimeout(hideTimer.current);
                hideTimer.current = setTimeout(() => {
                    if (playback.store.get().phase === 'playing' && options.current?.hidden)
                        setControlsHidden(true);
                }, 3000);
            }}
        >
            <video
                ref={video}
                id="video"
                playsInline
                aria-label="Diretta TV"
                onDoubleClick={() => void fullscreen()}
            />
            <div className="player-backdrop">
                <img src="/assets/symbol.svg" alt="" />
            </div>
            <div
                id="player-overlay"
                className={`player-overlay ${['error', 'interaction', 'restart'].includes(state.phase) ? 'error' : ''}`}
                role="status"
                hidden={!overlay}
            >
                <span
                    className="player-spinner"
                    hidden={!!state.widevine || state.phase === 'restart'}
                />
                <h2 id="player-status">{state.title}</h2>
                <p id="player-status-detail">{state.detail}</p>
                {state.widevine && (
                    <div
                        className="widevine-setup"
                        data-phase={state.phase === 'restart' ? 'restart' : state.widevine.phase}
                    >
                        <div
                            className="widevine-progress"
                            role="progressbar"
                            aria-label="Preparazione di Widevine"
                            aria-valuetext={state.title}
                            hidden={['error', 'restart'].includes(state.phase)}
                        >
                            <span />
                        </div>
                        <span className="widevine-elapsed">
                            {state.phase === 'restart'
                                ? 'Installazione completata'
                                : state.phase === 'error'
                                  ? 'Puoi riprovare il download'
                                  : `Tempo trascorso: ${Math.floor(state.setupElapsed / 60)}:${String(Math.floor(state.setupElapsed % 60)).padStart(2, '0')}`}
                        </span>
                    </div>
                )}
                <button
                    className="button primary"
                    id="player-retry"
                    hidden={state.phase !== 'error'}
                    onClick={() => state.current && void playback.load(state.current)}
                >
                    Riprova
                </button>
                <button
                    className="button primary"
                    id="player-start"
                    hidden={state.phase !== 'interaction'}
                    onClick={() => void playback.resume()}
                >
                    <Icon name="play" />
                    Avvia la diretta
                </button>
            </div>
            <div className="player-top">
                <span className="live-badge">
                    <span /> IN DIRETTA
                </span>
                <span id="player-channel-label">{state.channel?.name}</span>
                <button
                    className="icon-button player-close"
                    id="player-close"
                    aria-label="Chiudi la diretta"
                    title="Chiudi la diretta"
                    onClick={() => void playback.stop()}
                >
                    <Icon name="close" />
                </button>
            </div>
            <div className="player-controls" id="player-controls">
                <div className="live-track">
                    <span />
                    <button
                        id="go-live"
                        title="Torna alla diretta"
                        onClick={() => playback.goLive()}
                    >
                        DIRETTA <i />
                    </button>
                </div>
                <div className="control-row">
                    <button
                        className="icon-button"
                        id="play-pause"
                        aria-label={state.paused ? 'Riprendi' : 'Pausa'}
                        title="Pausa · Spazio"
                        onClick={() => playback.togglePause()}
                    >
                        <Icon name={state.paused ? 'play' : 'pause'} />
                    </button>
                    <button
                        className="icon-button"
                        id="mute"
                        aria-label={state.muted ? 'Attiva audio' : 'Disattiva audio'}
                        title="Audio · M"
                        onClick={toggleMute}
                    >
                        <Icon name={state.muted || state.volume === 0 ? 'muted' : 'volume'} />
                    </button>
                    <input
                        id="volume"
                        className="volume"
                        type="range"
                        min="0"
                        max="1"
                        step="0.01"
                        value={state.volume}
                        aria-label="Volume"
                        onChange={(event) => {
                            const volume = Number(event.target.value);
                            playback.volume(volume, false);
                            clearTimeout(volumeTimer.current);
                            pendingVolume.current = { volume, muted: false };
                            volumeTimer.current = setTimeout(() => {
                                void savePreferences(pendingVolume.current);
                                pendingVolume.current = null;
                            }, 250);
                        }}
                    />
                    <span className="control-channel" id="control-channel">
                        {state.channel?.name}
                    </span>
                    <span id="video-quality" className="video-quality">
                        {state.height ? `${state.height}p` : ''}
                    </span>
                    <button
                        className="icon-button"
                        id="player-settings"
                        aria-label="Opzioni di riproduzione"
                        title="Opzioni di riproduzione"
                        aria-expanded={expanded}
                        onClick={() => setExpanded(!expanded)}
                    >
                        <Icon name="settings" />
                    </button>
                    <button
                        className="icon-button"
                        id="picture-in-picture"
                        aria-label="Picture in picture"
                        title="Picture in picture"
                        hidden={!document.pictureInPictureEnabled}
                        onClick={async () => {
                            try {
                                if (document.pictureInPictureElement)
                                    await document.exitPictureInPicture();
                                else await video.current?.requestPictureInPicture();
                            } catch {
                                toast('Picture in picture non è disponibile per questa diretta.');
                            }
                        }}
                    >
                        <Icon name="pip" />
                    </button>
                    <button
                        className="icon-button"
                        id="fullscreen"
                        aria-label="Schermo intero"
                        title="Schermo intero · F"
                        onClick={() => void fullscreen()}
                    >
                        <Icon name="fullscreen" />
                    </button>
                </div>
            </div>
            <div ref={options} className="player-options" id="player-options" hidden>
                <h3>Riproduzione</h3>
                <label>
                    Qualità massima
                    <select
                        id="playback-quality"
                        value={preferences.quality}
                        onChange={(event) =>
                            void savePreferences({ quality: event.target.value as Quality })
                        }
                    >
                        <QualityOptions />
                    </select>
                </label>
                <label>
                    Audio
                    <select
                        id="audio-track"
                        value={state.audio}
                        onChange={(event) => playback.audio(event.target.value)}
                    >
                        {state.audioTracks.length ? (
                            state.audioTracks.map((t) => (
                                <option key={t.language} value={t.language}>
                                    {languageName(t.language)}
                                </option>
                            ))
                        ) : (
                            <option value="">Predefinito</option>
                        )}
                    </select>
                </label>
                <label>
                    Sottotitoli
                    <select
                        id="text-track"
                        value={state.text}
                        onChange={(event) => playback.text(event.target.value)}
                    >
                        <option value="off">Disattivati</option>
                        {state.textTracks.map((t) => (
                            <option key={t.id} value={String(t.id)}>
                                {t.label || t.language || 'Sottotitoli'}
                            </option>
                        ))}
                    </select>
                </label>
            </div>
            <div className="mini-controls">
                <button
                    id="mini-expand"
                    aria-label="Espandi il player"
                    onClick={() => navigate('watch')}
                >
                    <Icon name="minimize" />
                    <span id="mini-channel-name">{state.channel?.name}</span>
                </button>
                <button
                    className="icon-button"
                    id="mini-close"
                    aria-label="Chiudi il mini player"
                    onClick={() => void playback.stop()}
                >
                    <Icon name="close" />
                </button>
            </div>
        </div>
    );
}
