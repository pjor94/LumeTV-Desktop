const test = require('node:test');
const assert = require('node:assert/strict');
const { escapeText, searchable, schedule, currentProgramme, progress, selectProgrammes, filterChannels } = require('./app.js');

test('programme metadata is escaped before insertion into the UI', () => {
  assert.equal(escapeText('<img src=x onerror="secret"> & \'x\''), '&lt;img src=x onerror=&quot;secret&quot;&gt; &amp; &#39;x&#39;');
  assert.equal(searchable('Città · APERTA!'), 'citta aperta');
});

test('overlapping XMLTV entries use the newer programme and preserve the original data', () => {
  const original = [{ title: 'Prima', start: 100, end: 500 }, { title: 'Dopo', start: 300, end: 700 }];
  assert.equal(currentProgramme(original, 350).title, 'Dopo');
  assert.equal(schedule(original)[0].end, 300);
  assert.equal(original[0].end, 500);
  assert.equal(currentProgramme(original, 800), null);
  assert.equal(currentProgramme([{ title: 'EPG non disponibile', start: 100, end: 900 }], 400), null);
  assert.equal(progress(original[0], 50), 0);
  assert.equal(progress(original[0], 900), 100);
});

test('discovery periods use real guide intervals and apply favorites and accent-insensitive search', () => {
  const now = new Date(2026, 10, 8, 19, 0).getTime();
  const channels = [{ id: 'rai1', name: 'Rai 1', number: 1 }, { id: 'la7', name: 'La7', number: 7 }];
  const guide = {
    rai1: [{ title: 'Città aperta', start: now - 1000, end: now + 1000 }, { title: 'Stasera', start: now + 3_600_000, end: now + 7_200_000 }],
    la7: [{ title: 'Notizie', start: now - 5000, end: now + 5000 }],
  };
  assert.equal(selectProgrammes(channels, guide, { now, query: 'citta' })[0].programme.title, 'Città aperta');
  assert.deepEqual(selectProgrammes(channels, guide, { now, favoritesOnly: true, favorites: ['la7'] }).map(i => i.channel.id), ['la7']);
  assert.equal(selectProgrammes(channels, guide, { now, period: 'soon' })[0].programme.title, 'Stasera');
  assert.equal(selectProgrammes(channels, guide, { now, period: 'tonight' })[0].programme.title, 'Stasera');
  assert.deepEqual(selectProgrammes(channels, {}, { now }), []);
});

test('catalog filters combine provider, group, favorites and channel number search', () => {
  const channels = [{ id: 'rai1', name: 'Rai 1', number: 1, provider: 'rai', group: 'Generalisti' }, { id: 'la7', name: 'La7', number: 7, provider: 'la7', group: 'Generalisti' }];
  assert.deepEqual(filterChannels(channels, { provider: 'rai', favoritesOnly: true, favorites: ['la7'] }), []);
  assert.equal(filterChannels(channels, { query: '7' })[0].id, 'la7');
  assert.equal(filterChannels(channels, { group: 'Generalisti', provider: 'rai' }).length, 1);
});

test('Rai includes its regional channels while the remaining provider filters stay distinct', () => {
  const channels = [{id:'national',provider:'rai',name:'Rai 1'}, {id:'regional',provider:'rainewsrg',name:'TGR Lombardia'}, {id:'tv8',provider:'sky',name:'TV8'}, {id:'nove',provider:'discovery',name:'Nove'}, {id:'radio',provider:'direct',name:'Radio'}];
  assert.deepEqual(filterChannels(channels,{provider:'rai'}).map(c=>c.id),['national','regional']);
  for (const provider of ['sky','discovery','direct']) assert.equal(filterChannels(channels,{provider}).length,1);
});
