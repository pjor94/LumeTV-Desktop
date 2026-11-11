import { bounded } from './api';
import { createStore } from './store';
import { prepareWidevine, widevineMessage, WidevineSetupError } from './widevine';
import type { Capabilities } from './runtime';
import type { WidevineStatus } from './types';

export interface StartupState {
    phase: 'checking' | 'preparing' | 'ready' | 'error' | 'restart';
    title: string;
    detail: string;
    component: WidevineStatus | null;
    elapsed: number;
}

// Startup checks EME without a channel, media load or license request. The
// catalog becomes interactive only after the component is actually usable.
export class StartupController {
    readonly store = createStore<StartupState>({
        phase: 'checking',
        title: 'Prepariamo Lume.',
        detail: 'Verifichiamo il componente necessario per i canali protetti.',
        component: null,
        elapsed: 0,
    });
    private pending: Promise<void> | null = null;

    constructor(
        private readonly probe: (signal: AbortSignal) => Promise<Capabilities>,
        private readonly prepare: typeof prepareWidevine = prepareWidevine,
    ) {}

    start(): Promise<void> {
        if (this.pending) return this.pending;
        this.store.set({
            phase: 'checking',
            title: 'Controllo di Widevine.',
            detail: 'Verifichiamo il componente necessario per i canali protetti.',
            component: null,
            elapsed: 0,
        });
        const controller = new AbortController();
        this.pending = (async () => {
            try {
                const capabilities = await bounded(
                    this.probe(controller.signal),
                    12_000,
                    controller.signal,
                );
                if (!capabilities.avc_mse_supported || !capabilities.aac_mse_supported)
                    throw { code: 'codec_unavailable' };
                if (!capabilities.secure_context || !capabilities.eme_available)
                    throw { code: 'eme_unavailable' };
                const prepared = capabilities.widevine_available
                    ? capabilities
                    : await this.prepare(capabilities, {
                          signal: controller.signal,
                          probe: () => this.probe(controller.signal),
                          update: (component, elapsed) => {
                              const [title, detail] = widevineMessage(component);
                              this.store.set({
                                  phase: 'preparing',
                                  title,
                                  detail,
                                  component,
                                  elapsed,
                              });
                          },
                      });
                if (!prepared.widevine_available)
                    throw new WidevineSetupError('widevine_unavailable');
                this.store.set({
                    phase: 'ready',
                    title: 'Lume è pronta.',
                    detail: '',
                    component: null,
                });
            } catch (error) {
                controller.abort();
                const code = (error as { code?: string }).code;
                if (code === 'widevine_restart') {
                    this.store.set({
                        phase: 'restart',
                        title: 'Widevine è installato.',
                        detail: 'Per completare la preparazione su Linux, chiudi completamente Lume e riaprila. Il componente verrà caricato al prossimo avvio.',
                    });
                } else if (code === 'codec_unavailable' || code === 'eme_unavailable') {
                    this.store.set({
                        phase: 'error',
                        title: 'Il motore video non è pronto.',
                        detail: 'Questa installazione non rende disponibili i formati o i componenti necessari. Verifica di usare il pacchetto Lume completo per il tuo sistema, poi riprova.',
                    });
                } else {
                    this.store.set({
                        phase: 'error',
                        title: 'Non riusciamo a completare la preparazione.',
                        detail: 'Controlla la connessione e premi Riprova. Lume deve completare questa fase prima di aprire i canali.',
                    });
                }
            }
        })().finally(() => {
            this.pending = null;
        });
        return this.pending;
    }
}
