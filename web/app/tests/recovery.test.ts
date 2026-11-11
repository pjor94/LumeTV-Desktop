import { test } from 'node:test';
import assert from 'node:assert/strict';
import { RecoveryController } from '../src/recovery';

function harness() {
    let now = 0;
    const timers: { at: number; callback: () => void; active: boolean }[] = [];
    let retries = 0,
        exhausted = 0;
    const waiting: { attempt: number; offline: boolean }[] = [];
    const controller = new RecoveryController({
        online: () => true,
        retry: () => retries++,
        exhausted: () => exhausted++,
        waiting: (attempt, offline) => waiting.push({ attempt, offline }),
        later: (callback, delay) => {
            const timer = { at: now + delay, callback, active: true };
            timers.push(timer);
            return () => {
                timer.active = false;
            };
        },
    });
    const advance = (milliseconds: number) => {
        const end = now + milliseconds;
        while (true) {
            const next = timers
                .filter((t) => t.active && t.at <= end)
                .sort((a, b) => a.at - b.at)[0];
            if (!next) break;
            now = next.at;
            next.active = false;
            next.callback();
        }
        now = end;
    };
    controller.start();
    return { controller, advance, waiting, retries: () => retries, exhausted: () => exhausted };
}
test('recovery waits for the network and schedules a single retry when it returns', () => {
    const h = harness();
    h.controller.networkChanged(false);
    h.controller.failed(true);
    h.advance(60_000);
    assert.equal(h.retries(), 0);
    assert.equal(h.waiting.at(-1)?.offline, true);
    h.controller.networkChanged(true);
    h.advance(0);
    assert.equal(h.retries(), 1);
});
test('the retry budget is finite and brief playback does not reset it', () => {
    const h = harness();
    for (const delay of [2000, 5000, 10000]) {
        h.controller.failed(true);
        h.advance(delay);
        h.controller.playing();
        h.advance(500);
    }
    h.controller.failed(true);
    assert.equal(h.retries(), 3);
    assert.equal(h.exhausted(), 1);
});
test('pausing or changing channel cancels retries and stalled-playback checks', () => {
    const h = harness();
    h.controller.buffering();
    h.advance(45_000);
    h.controller.stop();
    h.controller.networkChanged(true);
    h.advance(100_000);
    assert.equal(h.retries(), 0);
});
test('stable playback grants a new budget while fatal failures never retry', () => {
    const h = harness();
    h.controller.failed(true);
    h.advance(2000);
    h.controller.playing();
    h.advance(30_000);
    h.controller.failed(true);
    h.advance(2000);
    assert.equal(h.retries(), 2);
    h.controller.failed(false);
    h.advance(60_000);
    assert.equal(h.retries(), 2);
    assert.equal(h.exhausted(), 1);
});
