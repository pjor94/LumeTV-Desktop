type Cancel = () => void;
interface RecoveryDependencies {
    retry(): void;
    waiting(attempt: number, offline: boolean): void;
    exhausted(): void;
    online?(): boolean;
    later?(callback: () => void, milliseconds: number): Cancel;
}
const delays = [2000, 5000, 10000];
export class RecoveryController {
    private active = false;
    private attempts = 0;
    private failure = false;
    private online = true;
    private retryTimer?: Cancel;
    private stallTimer?: Cancel;
    private stableTimer?: Cancel;
    constructor(private readonly deps: RecoveryDependencies) {}
    get pending() {
        return this.failure && this.active;
    }
    private later(callback: () => void, milliseconds: number) {
        if (this.deps.later) return this.deps.later(callback, milliseconds);
        const timer = setTimeout(callback, milliseconds);
        return () => clearTimeout(timer);
    }
    start() {
        this.stop();
        this.active = true;
        this.attempts = 0;
        this.online = this.deps.online?.() ?? navigator.onLine;
    }
    stop() {
        this.active = false;
        this.failure = false;
        this.retryTimer?.();
        this.stallTimer?.();
        this.stableTimer?.();
        this.retryTimer = this.stallTimer = this.stableTimer = undefined;
    }
    failed(recoverable: boolean) {
        if (!this.active) return;
        this.stallTimer?.();
        this.stallTimer = undefined;
        this.stableTimer?.();
        this.stableTimer = undefined;
        if (!recoverable || this.attempts >= delays.length) {
            this.stop();
            this.deps.exhausted();
            return;
        }
        this.failure = true;
        if (!this.retryTimer) this.schedule();
    }
    private schedule(immediate = false) {
        this.deps.waiting(this.attempts + 1, !this.online);
        if (!this.online) return;
        this.retryTimer = this.later(
            () => {
                this.retryTimer = undefined;
                if (!this.active) return;
                this.attempts++;
                this.deps.retry();
            },
            immediate ? 0 : delays[this.attempts],
        );
    }
    networkChanged(online: boolean) {
        this.online = online;
        if (!this.active || !this.failure) return;
        this.retryTimer?.();
        this.retryTimer = undefined;
        this.schedule(online);
    }
    buffering() {
        this.stableTimer?.();
        this.stableTimer = undefined;
        if (this.active && !this.failure && !this.stallTimer)
            this.stallTimer = this.later(() => {
                this.stallTimer = undefined;
                this.failed(true);
            }, 45_000);
    }
    playing() {
        if (!this.active) return;
        this.retryTimer?.();
        this.retryTimer = undefined;
        this.stallTimer?.();
        this.stallTimer = undefined;
        this.failure = false;
        if (!this.stableTimer)
            this.stableTimer = this.later(() => {
                this.attempts = 0;
                this.stableTimer = undefined;
            }, 30_000);
    }
}
