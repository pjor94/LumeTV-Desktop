import test from 'node:test';
import assert from 'node:assert/strict';
import { ContinuousData } from '../src/personal';
const settle = () => new Promise((resolve) => setTimeout(resolve, 5));

test('continuous catalog keeps three blocks and reads positions outside the initial preview', async () => {
    const requested: number[] = [];
    const load = async (offset: number) => {
        requested.push(offset);
        return { rows: Array.from({ length: 200 }, (_, i) => offset + i), total: 1_000_000 };
    };
    const catalog = new ContinuousData(200, load);
    catalog.bind(load);
    await settle();
    catalog.viewport(499900);
    await settle();
    assert.equal(catalog.get(499999), 499999);
    assert.ok(catalog.cachedRows <= 600);
    catalog.viewport(999900);
    await settle();
    assert.equal(catalog.get(999999), 999999);
    assert.ok(catalog.cachedRows <= 600);
    assert.ok(requested.includes(999800));
});
test('a late response from an obsolete filter cannot overwrite the new catalog', async () => {
    let resolveOld!: (page: { rows: number[]; total: number }) => void;
    const catalog = new ContinuousData<number>(
        50,
        () =>
            new Promise((resolve) => {
                resolveOld = resolve;
            }),
    );
    catalog.bind(
        () =>
            new Promise((resolve) => {
                resolveOld = resolve;
            }),
    );
    catalog.bind(async () => ({ rows: [42], total: 1 }));
    await settle();
    resolveOld({ rows: [1], total: 999999 });
    await settle();
    assert.equal(catalog.total, 1);
    assert.equal(catalog.get(0), 42);
    catalog.close();
});
test('failed windows offer retry without throwing away already loaded rows', async () => {
    let fail = true;
    const load = async (offset: number) => {
        if (offset === 50 && fail) throw new Error('fixture');
        return { rows: [offset], total: 100 };
    };
    const catalog = new ContinuousData(50, load);
    catalog.bind(load);
    await settle();
    catalog.viewport(50);
    await settle();
    assert.equal(catalog.get(0), 0);
    assert.ok(catalog.error);
    fail = false;
    catalog.retry();
    await settle();
    assert.equal(catalog.get(50), 50);
});
test('background refresh updates visible programmes while retaining the current catalog', async () => {
    let version = 1;
    let release!: (page: { rows: number[]; total: number }) => void;
    const load = async (offset: number) =>
        version === 1
            ? { rows: [offset + 1], total: 100 }
            : new Promise<{ rows: number[]; total: number }>((resolve) => {
                  release = resolve;
              });
    const catalog = new ContinuousData(50, load);
    catalog.bind(load);
    await settle();
    version = 2;
    catalog.refresh();
    await settle();
    assert.equal(catalog.get(0), 1);
    release({ rows: [2], total: 100 });
    await settle();
    assert.equal(catalog.get(0), 2);
    assert.equal(catalog.total, 100);
    catalog.close();
});
