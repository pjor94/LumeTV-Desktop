import test from 'node:test';
import assert from 'node:assert/strict';
import {
    currentProgramme,
    schedule,
    searchable,
    progress,
    filterChannels,
    selectProgrammes,
} from '../src/domain';
import type { Channel } from '../src/types';
const channel = (id: string, provider = id): Channel => ({
    id,
    provider,
    number: 1,
    name: id === 'rai' ? 'Rai 1' : id,
    group: 'Generalisti',
    color: '#554ee5',
});

test('newer overlapping EPG entries win without changing the source schedule', () => {
    const source = [
        { title: 'Prima', start: 100, end: 500 },
        { title: 'Dopo', start: 300, end: 700 },
    ];
    assert.equal(currentProgramme(source, 350)?.title, 'Dopo');
    assert.equal(schedule(source)[0].end, 300);
    assert.equal(source[0].end, 500);
    assert.equal(currentProgramme(source, 800), null);
    assert.equal(
        currentProgramme([{ title: 'EPG non disponibile', start: 1, end: 900 }], 100),
        null,
    );
    assert.equal(progress(source[0], 50), 0);
    assert.equal(progress(source[0], 900), 100);
});
test('search normalizes accents and combines channel number, provider, group and favorites', () => {
    const rai = channel('rai');
    const la7 = { ...channel('la7'), name: 'La7', number: 7 };
    assert.equal(searchable('Città · APERTA!'), 'citta aperta');
    assert.equal(filterChannels([rai, la7], { query: '7' })[0].id, 'la7');
    assert.deepEqual(
        filterChannels([rai, la7], { provider: 'rai', favoritesOnly: true, favorites: ['la7'] }),
        [],
    );
    assert.equal(
        filterChannels([rai], {
            query: 'citta',
            programmes: { rai: [{ title: 'Città aperta', start: 1, end: 900 }] },
            now: 100,
        }).length,
        1,
    );
});
test('Rai filters include regional channels and other providers remain distinct', () => {
    const channels = ['rai', 'rainewsrg', 'sky', 'discovery', 'direct'].map((id) => channel(id));
    assert.deepEqual(
        filterChannels(channels, { provider: 'rai' }).map((c) => c.id),
        ['rai', 'rainewsrg'],
    );
    for (const provider of ['sky', 'discovery', 'direct'])
        assert.equal(filterChannels(channels, { provider }).length, 1);
});
test('discovery uses actual programme intervals for now, soon and tonight', () => {
    const now = new Date(2026, 10, 11, 19, 0).getTime();
    const channels = [channel('rai'), channel('la7')];
    const guide = {
        rai: [
            { title: 'Città aperta', start: now - 1000, end: now + 1000 },
            { title: 'Stasera', start: now + 3_600_000, end: now + 7_200_000 },
        ],
        la7: [{ title: 'Notizie', start: now - 1000, end: now + 1000 }],
    };
    assert.equal(
        selectProgrammes(channels, guide, { now, query: 'citta' })[0].programme.title,
        'Città aperta',
    );
    assert.equal(
        selectProgrammes(channels, guide, { now, period: 'soon' })[0].programme.title,
        'Stasera',
    );
    assert.equal(
        selectProgrammes(channels, guide, { now, period: 'tonight' })[0].programme.title,
        'Stasera',
    );
    assert.deepEqual(
        selectProgrammes(channels, guide, { now, favoritesOnly: true, favorites: ['la7'] }).map(
            (x) => x.channel.id,
        ),
        ['la7'],
    );
    assert.deepEqual(selectProgrammes(channels, {}, { now }), []);
});
