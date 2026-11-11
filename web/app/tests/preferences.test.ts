import test from 'node:test';
import assert from 'node:assert/strict';
import { createPreferenceQueue } from '../src/api';

test('history and fast preference edits are serialized and applied in order', async () => {
    const requests: string[] = [];
    const responses: number[] = [];
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
        release = resolve;
    });
    const save = createPreferenceQueue<number>(
        async (path) => {
            requests.push(path);
            if (requests.length === 1) await gate;
            return requests.length;
        },
        (value) => responses.push(value),
    );
    const history = save({}, '/api/played');
    const favorite = save({ toggle_favorite: 'rai1' });
    await Promise.resolve();
    await Promise.resolve();
    assert.deepEqual(requests, ['/api/played']);
    release();
    await Promise.all([history, favorite]);
    assert.deepEqual(requests, ['/api/played', '/api/preferences']);
    assert.deepEqual(responses, [1, 2]);
});
test('a failed write is reported to its caller and does not poison subsequent writes', async () => {
    let calls = 0;
    const save = createPreferenceQueue(
        async () => {
            if (++calls === 1) throw Error('offline');
            return 'saved';
        },
        () => {},
    );
    await assert.rejects(save({}), /offline/);
    assert.equal(await save({}), 'saved');
});
