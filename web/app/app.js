(function (global) {
  'use strict';

  const escapeText = value => String(value == null ? '' : value).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const searchable = value => String(value || '').normalize('NFD').replace(/\p{M}/gu, '').toLowerCase().replace(/[^\p{L}\p{N}]+/gu, ' ').trim();

  function schedule(programmes) {
    const sorted = (programmes || []).filter(p => p && Number.isFinite(p.start) && Number.isFinite(p.end) && p.end > p.start &&
      !/^EPG\s+(non disponibile|not available)$/i.test(String(p.title || '').trim()))
      .slice().sort((a, b) => a.start - b.start);
    return sorted.map((p, index) => ({ ...p, end: Math.min(p.end, sorted[index + 1]?.start || p.end) })).filter(p => p.end > p.start);
  }

  function currentProgramme(programmes, now = Date.now()) {
    return schedule(programmes).find(p => p.start <= now && p.end > now) || null;
  }

  function progress(programme, now = Date.now()) {
    return programme && programme.end > programme.start
      ? Math.max(0, Math.min(100, (now - programme.start) / (programme.end - programme.start) * 100)) : 0;
  }

  function selectProgrammes(channels, programmes, { period = 'now', favorites = [], favoritesOnly = false, query = '', now = Date.now() } = {}) {
    const words = searchable(query).split(' ').filter(Boolean);
    const evening = new Date(now); evening.setHours(20, 30, 0, 0);
    const midnight = new Date(now); midnight.setHours(24, 0, 0, 0);
    return channels.filter(c => !favoritesOnly || favorites.includes(c.id)).flatMap(channel => {
      const shows = schedule(programmes[channel.id]);
      const candidates = period === 'tonight'
        ? shows.filter(p => p.start < midnight.getTime() && p.end > evening.getTime() && p.end > now).slice(0, 2)
        : period === 'soon'
          ? shows.filter(p => p.start > now && p.start <= now + 90 * 60_000).slice(0, 1)
          : shows.filter(p => p.start <= now && p.end > now).slice(-1);
      return candidates.filter(p => words.every(word => searchable(`${channel.name} ${p.title} ${p.description}`).includes(word)))
        .map(programme => ({ channel, programme, key: `${channel.id}~${programme.start}` }));
    }).sort((a, b) => {
      if (period !== 'now' && a.programme.start !== b.programme.start) return a.programme.start - b.programme.start;
      return Number(favorites.includes(b.channel.id)) - Number(favorites.includes(a.channel.id)) || a.channel.number - b.channel.number;
    });
  }

  const matchesProvider = (channel, provider) => provider === 'all' || channel.provider === provider || (provider === 'rai' && channel.provider === 'rainewsrg');

  function filterChannels(channels, { provider = 'all', group = 'all', favoritesOnly = false, favorites = [], query = '', programmes = {} } = {}) {
    const words = searchable(query).split(' ').filter(Boolean);
    return channels.filter(c => matchesProvider(c, provider) && (group === 'all' || c.group === group) &&
      (!favoritesOnly || favorites.includes(c.id)) && words.every(word => searchable(`${c.name} ${c.number} ${c.group} ${currentProgramme(programmes[c.id])?.title || ''}`).includes(word)));
  }

  const helpers = { escapeText, searchable, schedule, currentProgramme, progress, selectProgrammes, filterChannels, matchesProvider };
  if (typeof module === 'object' && module.exports) module.exports = helpers;
  if (!global.document) return;

  const doc = global.document;
  const ui = global.LumeUI;
  ui.initialize(doc);
  const drmSetupSeconds = 300;
  const $ = id => doc.getElementById(id);
  const icon = (name, className = '') => `<svg class="${className}" aria-hidden="true"><use href="#i-${name}"/></svg>`;
  const time = value => new Intl.DateTimeFormat('it-IT', { hour: '2-digit', minute: '2-digit' }).format(new Date(value));
  const date = value => new Intl.DateTimeFormat('it-IT', { weekday: 'long', day: 'numeric', month: 'long' }).format(new Date(value));
  const range = p => `${time(p.start)} – ${time(p.end)}`;
  const providerName = value => ({ rai: 'RAI', rainewsrg: 'RAI TGR', mediaset: 'MEDIASET', la7: 'LA7', sky: 'SKY', discovery: 'DISCOVERY', direct: 'ALTRI CANALI' }[value] || value);
  const state = { channels: [], preferences: { favorites: [], recent: [], reminders: [], theme: 'dark', volume: .7, muted: false, quality: 'auto' },
    catalog: { updated_at: 0, loading: false, error: null, revision: '', next_retry_at: 0 },
    guide: { programmes: {}, logos: {}, updated_at: 0, loading: true }, page: 'channels', previousPage: 'channels', query: '', provider: 'all', group: 'all',
    period: 'now', favoritesOnly: false, discoveryKey: null, guideStart: Math.floor(Date.now() / 1_800_000) * 1_800_000,
    current: null, session: null, pending: null, watchFavorites: false, phase: 'idle', generation: 0, version: '', os: '', playedId: null };
  let token = '';
  let player = null;
  let playerPromise = null;
  let loadController = null;
  let hideControlsTimer = null;
  let guidePoll = null;
  let catalogPoll = null;
  let sampleTimer = null;
  let volumeSaveTimer = null;
  let preferenceQueue = Promise.resolve();
  const reminderShown = new Set();

  function apiFetch(path, options = {}) {
    return global.fetch(path, { ...options, cache: 'no-store', credentials: 'omit',
      headers: { ...(options.headers || {}), 'X-Lume-Token': token } });
  }
  async function api(path, body, options = {}) {
    const response = await apiFetch(path, { ...options, ...(body === undefined ? {} : {
      method: 'POST', headers: { 'Content-Type': 'application/json', ...(options.headers || {}) }, body: JSON.stringify(body),
    }) });
    if (!response.ok) {
      const error = new Error('Local request failed');
      error.status = response.status;
      try { error.code = (await response.json()).error; } catch (_) { /* numeric transport status is enough */ }
      throw error;
    }
    return response.json();
  }

  async function bounded(promise, milliseconds, signal) {
    let timeout;
    let abort;
    try {
      return await Promise.race([promise, new Promise((_, reject) => {
        timeout = global.setTimeout(() => reject({ code: 'startup_timeout' }), milliseconds);
        abort = () => reject(new DOMException('Cancelled', 'AbortError'));
        if (signal?.aborted) abort(); else signal?.addEventListener('abort', abort, { once: true });
      })]);
    } finally {
      global.clearTimeout(timeout);
      signal?.removeEventListener('abort', abort);
    }
  }

  function toast(message, action) {
    const element = doc.createElement('div'); element.className = 'toast';
    if (!ui.canMove()) element.setAttribute('data-instant', '');
    element.innerHTML = icon(action ? 'bell' : 'check');
    const text = doc.createElement('span'); text.textContent = message; element.append(text);
    if (action) {
      const button = doc.createElement('button'); button.textContent = action.label;
      button.addEventListener('click', () => { action.run(); element.remove(); }); element.append(button);
    }
    $('toast-region').append(element);
    while ($('toast-region').children.length > 3) $('toast-region').firstElementChild.remove();
    global.setTimeout(() => { element.classList.add('removing'); global.setTimeout(() => element.remove(), 160); }, action ? 12_000 : 3500);
  }

  function savePreferences(patch, message, repaint = true) {
    // Serialize partial updates so quick clicks cannot overwrite newer preferences.
    preferenceQueue = preferenceQueue.catch(() => {}).then(async () => {
      state.preferences = await api('/api/preferences', patch);
      applyPreferences();
      if (repaint) render();
      if (message) toast(message);
      return state.preferences;
    }).catch(() => { toast('Non riesco a salvare le modifiche. Riprova.'); });
    return preferenceQueue;
  }

  function applyPreferences() {
    const theme = state.preferences.theme === 'system'
      ? (global.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light') : state.preferences.theme;
    doc.documentElement.dataset.theme = theme;
    $('volume').value = state.preferences.volume;
    $('playback-quality').value = state.preferences.quality;
    const video = $('video');
    video.volume = state.preferences.volume;
    video.muted = state.preferences.muted;
    updateVolumeIcon();
    applyQuality();
  }

  function stationLogo(channel, extra = '') {
    const name = channel.name.replace(/^Rai\s*/, '');
    const fallback = channel.provider === 'rai' && /^[12345]$/.test(name)
      ? `<span class="rai-word">Rai<b>${escapeText(name)}</b></span>`
      : `<span class="station-fallback ${channel.name.length > 7 ? 'small' : ''}">${escapeText(channel.name)}</span>`;
    return `<span class="station-logo ${extra}" style="--station:${channel.color}" aria-hidden="true">${fallback}${state.guide.logos[channel.id]
      ? `<img src="/art/channel/${encodeURIComponent(channel.id)}?v=${state.guide.updated_at}" alt="" loading="lazy" decoding="async">` : ''}</span>`;
  }

  function favoriteButton(channel, className = 'favorite-button') {
    const favorite = state.preferences.favorites.includes(channel.id);
    return `<button class="icon-button ${className}" data-favorite="${channel.id}" aria-pressed="${favorite}" aria-label="${favorite ? 'Rimuovi' : 'Aggiungi'} ${escapeText(channel.name)} ${favorite ? 'dai' : 'ai'} preferiti" title="${favorite ? 'Rimuovi dai preferiti' : 'Aggiungi ai preferiti'}">${icon('star')}</button>`;
  }

  function channelCard(channel) {
    const programme = currentProgramme(state.guide.programmes[channel.id]);
    return `<article data-key="channel-${channel.id}" class="channel-card ${state.current === channel.id ? 'is-playing' : ''}" style="--station:${channel.color}">
      <button class="channel-main" data-play="${channel.id}" data-focus="channel-${channel.id}" aria-label="Guarda ${escapeText(channel.name)}${programme ? ', ' + escapeText(programme.title) : ''}">
        <span class="channel-number">${String(channel.number).padStart(2, '0')} <span>/</span> ${providerName(channel.provider)}</span>
        ${stationLogo(channel)}<span class="channel-name">${escapeText(channel.name)}</span>
        <span class="channel-programme">${programme ? escapeText(programme.title) : 'Guarda in diretta'}</span>
        ${programme ? `<span class="channel-progress"><i style="width:${progress(programme)}%"></i></span>` : ''}
        <span class="channel-play-hint">${icon('play')}</span>
      </button>${favoriteButton(channel)}</article>`;
  }

  function heading(title, subtitle, eyebrow = '') {
    return `<div data-key="heading" class="page-heading"><div>${eyebrow ? `<div class="eyebrow">${eyebrow}</div>` : ''}<h1>${escapeText(title)}</h1><p>${escapeText(subtitle)}</p></div><span class="heading-date">${date(Date.now())}</span></div>`;
  }

  function emptyState(symbol, title, description, action) {
    return `<div data-key="empty" class="empty-state"><div class="empty-symbol">${icon(symbol)}</div><h2>${escapeText(title)}</h2><p>${escapeText(description)}</p>${action || ''}</div>`;
  }

  function hero() {
    const preferred = state.preferences.recent[0] || 'rai1';
    const channel = state.channels.find(c => c.id === preferred) || state.channels[0];
    if (!channel) return '';
    const programme = currentProgramme(state.guide.programmes[channel.id]);
    return `<section data-key="hero" class="hero" aria-label="Canale in evidenza"><div class="hero-content"><div class="hero-channel">${escapeText(channel.name)}${programme ? ' · In onda adesso' : ' · In diretta'}</div>
      <h2>${programme ? escapeText(programme.title) : 'Accendi il momento giusto.'}</h2>
      <p>${programme ? escapeText(programme.description || `La diretta di ${channel.name}, da vivere adesso.`) : 'I canali che ami, la guida per scegliere. Tutto il piacere della TV, con la semplicità di Lume.'}</p>
      <div class="hero-actions"><button class="button primary" data-play="${channel.id}">${icon('play')}Guarda ${programme ? 'ora' : escapeText(channel.name)}</button>${programme ? `<span class="hero-time">${range(programme)}</span>` : `<button class="text-button" data-route="discover" style="color:#b7c9e3">Cosa guardare ${icon('arrow')}</button>`}</div></div>
      ${stationLogo(channel, 'hero-station')}<img class="hero-mark" src="/assets/symbol.svg" alt=""></section>`;
  }

  function renderChannels(favoritesOnly = false) {
    const channels = filterChannels(state.channels, { provider: state.provider, group: state.group, favoritesOnly,
      favorites: state.preferences.favorites, query: state.query, programmes: state.guide.programmes });
    const recent = state.preferences.recent.map(id => state.channels.find(c => c.id === id)).filter(Boolean);
    const groups = Array.from(new Set(state.channels.map(c => c.group)));
    return heading(favoritesOnly ? 'I tuoi, sempre in prima fila.' : 'La TV, fatta semplice.', favoritesOnly
      ? 'I canali che ami. Un piccolo gesto, tutto il tuo mondo.' : `${state.channels.length} canali. Scegli il tuo, al resto pensa Lume.`)
      + (!favoritesOnly && !state.query && state.provider === 'all' && state.group === 'all' ? hero() : '')
      + (!favoritesOnly && !state.query && recent.length ? `<section data-key="recent" class="recent-strip" aria-label="Visti di recente"><span class="recent-label">Visti di recente</span><div class="recent-row">${recent.slice(0, 5).map(c => {
        const p = currentProgramme(state.guide.programmes[c.id]);
        return `<button class="recent-channel" data-play="${c.id}">${stationLogo(c)}<span><strong>${escapeText(c.name)}</strong><small>${p ? escapeText(p.title) : 'Torna alla diretta'}</small></span></button>`;
      }).join('')}</div></section>` : '')
      + `<div data-key="channel-heading" class="section-heading"><h2>${favoritesOnly ? 'I tuoi preferiti' : 'Tutti i tuoi canali'} <span>${channels.length}</span></h2>${!favoritesOnly ? `<button class="text-button" data-route="guide">Apri la guida ${icon('arrow')}</button>` : ''}</div>`
      + `<div data-key="filters" class="filter-bar"><div class="segmented provider-options" aria-label="Filtra per emittente">${[['all', 'Tutti'], ['rai', 'Rai'], ['mediaset', 'Mediaset'], ['la7', 'La7'], ['sky', 'Sky'], ['discovery', 'Discovery'], ['direct', 'Altri']].filter(([id]) => id === 'all' || state.channels.some(c => matchesProvider(c, id))).map(([id, name]) => `<button data-provider="${id}" aria-pressed="${state.provider === id}">${name}</button>`).join('')}</div><select class="category-select" id="group-filter" aria-label="Categoria"><option value="all">Tutte le categorie</option>${groups.map(group => `<option value="${escapeText(group)}" ${state.group === group ? 'selected' : ''}>${escapeText(group)}</option>`).join('')}</select><span class="filter-count">${channels.length} ${channels.length === 1 ? 'canale' : 'canali'}</span></div>`
      + (channels.length ? `<div data-key="channels" class="channel-grid">${channels.map(channelCard).join('')}</div>` : emptyState(favoritesOnly ? 'star' : 'search', favoritesOnly && !state.query ? 'Fai spazio ai tuoi preferiti.' : 'Nessun canale trovato.', favoritesOnly && !state.query
        ? 'Premi la stella su una scheda: ritroverai qui i tuoi canali, pronti da guardare.' : 'Prova un altro nome o modifica i filtri.', `<button class="button secondary" data-action="reset-filters">${favoritesOnly ? 'Esplora i canali' : 'Mostra tutti i canali'}</button>`));
  }

  function discovered() {
    return selectProgrammes(state.channels, state.guide.programmes, { period: state.period, favorites: state.preferences.favorites, favoritesOnly: state.favoritesOnly, query: state.query });
  }

  function renderDiscovery() {
    const items = discovered();
    const selected = items.find(i => i.key === state.discoveryKey) || items[0];
    state.discoveryKey = selected?.key || null;
    const introduction = { now: 'In onda adesso. Trova il programma che fa per te.', soon: 'Il prossimo momento da non perdere.', tonight: 'La serata comincia con una buona scelta.' }[state.period];
    return heading('Cosa guardare.', introduction)
      + `<div data-key="periods" class="discovery-periods"><div class="segmented" aria-label="Quando guardare">${[['now', 'Ora'], ['soon', 'Tra poco'], ['tonight', 'Stasera']].map(([id, label]) => `<button data-period="${id}" aria-pressed="${state.period === id}">${label}</button>`).join('')}</div><button class="favorites-filter" data-action="discover-favorites" aria-pressed="${state.favoritesOnly}">${icon('star')}Solo preferiti</button></div>`
      + (selected ? `<div data-key="discovery" class="discovery-layout"><section class="discovery-feature" style="--station:${selected.channel.color}">${stationLogo(selected.channel)}<div class="eyebrow">${selected.programme.start <= Date.now() ? 'IN ONDA ADESSO' : 'DA NON PERDERE'} · ${escapeText(selected.channel.name)}</div><h2>${escapeText(selected.programme.title)}</h2><div class="programme-meta"><span>${range(selected.programme)}</span><span>${remainingLabel(selected.programme)}</span></div>${selected.programme.start <= Date.now() ? `<span class="channel-progress"><i style="width:${progress(selected.programme)}%;background:var(--accent)"></i></span>` : ''}<p class="programme-description">${escapeText(selected.programme.description || 'La descrizione di questo programma non è disponibile nella guida.')}</p><div class="dialog-actions"><button class="button primary" data-play="${selected.channel.id}">${icon('play')}Guarda ${selected.programme.start <= Date.now() ? 'ora' : 'il canale'}</button><button class="icon-button" data-programme="${selected.key}" aria-label="Dettagli del programma" title="Dettagli del programma">${icon('info')}</button>${favoriteButton(selected.channel, '')}</div></section><div data-key="programmes" class="discovery-list" aria-label="Programmi">${items.map(item => `<button class="discovery-item" data-discover="${item.key}" data-focus="discover-${item.key}" aria-pressed="${item.key === selected.key}">${stationLogo(item.channel)}<span class="item-description"><strong>${escapeText(item.programme.title)}</strong><small>${escapeText(item.channel.name)} · ${range(item.programme)}</small></span><span class="remaining">${remainingLabel(item.programme)}</span></button>`).join('')}</div></div>`
        : emptyState(state.guide.loading ? 'clock' : 'compass', state.guide.loading ? 'Stiamo preparando la tua guida.' : 'Qui c’è spazio per una nuova scoperta.', state.guide.loading
          ? 'I programmi stanno arrivando. Nel frattempo puoi già guardare i tuoi canali.' : state.guide.error || 'Non ci sono programmi per questi filtri. Prova un altro orario o esplora tutti i canali.', `<button class="button secondary" data-route="channels">Esplora i canali</button>`));
  }

  function remainingLabel(p) {
    const now = Date.now();
    if (p.start > now) return `Tra ${Math.max(1, Math.ceil((p.start - now) / 60_000))} min`;
    return `Restano ${Math.max(0, Math.ceil((p.end - now) / 60_000))} min`;
  }

  function renderGuide() {
    const channels = filterChannels(state.channels, { favoritesOnly: state.favoritesOnly, favorites: state.preferences.favorites, query: state.query, programmes: state.guide.programmes });
    const start = state.guideStart, end = start + 4 * 3_600_000, width = 912;
    const x = value => (value - start) / 60_000 * 3.8;
    const nowX = x(Date.now());
    const nowStyle = nowX >= 0 && nowX <= width ? `--now-x:${nowX}px` : '';
    const ticks = Array.from({ length: 8 }, (_, i) => `<span style="left:${i * 114}px">${time(start + i * 1_800_000)}</span>`).join('');
    return heading('La tua serata, a colpo d’occhio.', 'Scorri la guida. Scegli un programma. Trova il tuo momento.')
      + `<div data-key="guide-toolbar" class="guide-toolbar"><div class="segmented" aria-label="Canali nella guida"><button data-guide-favorites="false" aria-pressed="${!state.favoritesOnly}">Tutti</button><button data-guide-favorites="true" aria-pressed="${state.favoritesOnly}">Preferiti</button></div><span class="guide-spacer"></span><span class="guide-date">${date(start)}</span><button class="button secondary" data-guide-time="now">Adesso</button><button class="button secondary" data-guide-time="tonight">Stasera</button><button class="icon-button" data-guide-shift="-1" aria-label="Quattro ore prima" title="Quattro ore prima">${icon('chevron', 'reverse-chevron')}</button><button class="icon-button" data-guide-shift="1" aria-label="Quattro ore dopo" title="Quattro ore dopo">${icon('chevron')}</button></div>`
      + (channels.length ? `<div class="guide-scroll" id="guide-scroll"><div class="guide-row guide-header"><span class="guide-channel">CANALI</span><div class="guide-times" style="width:${width}px;${nowStyle}">${ticks}</div></div>${channels.map(channel => {
        const shows = schedule(state.guide.programmes[channel.id]).filter(p => p.end > start && p.start < end);
        return `<div data-key="guide-${channel.id}" class="guide-row"><button class="guide-channel" data-play="${channel.id}" aria-label="Guarda ${escapeText(channel.name)}"><span class="guide-channel-number">${String(channel.number).padStart(2, '0')}</span>${stationLogo(channel)}<strong>${escapeText(channel.name)}</strong></button><div class="programme-track" style="width:${width}px;${nowStyle}">${shows.length ? shows.map(p => {
          const left = Math.max(0, x(p.start)), right = Math.min(width, x(p.end));
          return `<button class="programme-block ${p.start <= Date.now() && p.end > Date.now() ? 'live' : ''}" data-programme="${channel.id}~${p.start}" data-focus="guide-${channel.id}-${p.start}" style="left:${left + 3}px;width:${Math.max(3, right - left - 6)}px" aria-label="${escapeText(p.title)}, ${escapeText(channel.name)}, ${range(p)}"><strong>${escapeText(p.title)}</strong><small>${range(p)}</small></button>`;
        }).join('') : `<span class="programme-gap">${state.guide.loading ? 'Caricamento della guida…' : 'Programmazione non disponibile per questo orario'}</span>`}</div></div>`;
      }).join('')}</div>` : emptyState('guide', 'Nessun canale in questa guida.', 'Aggiungi un preferito o modifica i filtri per ritrovare i tuoi programmi.'))
      + (state.guide.error ? `<p class="notice">${escapeText(state.guide.error)}</p>` : '');
  }

  function renderSettings() {
    const p = state.preferences;
    const row = (symbol, title, description, control) => `<div class="settings-row"><span class="settings-icon">${icon(symbol)}</span><div class="settings-label"><strong>${title}</strong><p>${description}</p></div>${control}</div>`;
    return heading('Lume, a modo tuo.', 'I piccoli dettagli che rendono la TV ancora più tua.')
      + `<div class="settings-layout"><section class="settings-section"><h2>Il tuo spazio</h2><div class="settings-card">`
      + row('moon', 'Aspetto', 'Scegli la luce giusta per il tuo momento.', `<div class="segmented" aria-label="Aspetto">${[['dark', 'Scuro'], ['light', 'Chiaro'], ['system', 'Sistema']].map(([id, label]) => `<button data-theme="${id}" aria-pressed="${p.theme === id}">${label}</button>`).join('')}</div>`)
      + row('star', 'Cambia canale tra i preferiti', 'Usa i tasti ↑ e ↓ per passare solo tra i canali che ami.', `<button class="toggle" role="switch" aria-label="Cambia canale solo tra i preferiti" aria-checked="${p.zap_favorites_only}" data-setting-toggle="zap_favorites_only"></button>`)
      + `</div></section><section class="settings-section"><h2>La tua visione</h2><div class="settings-card">`
      + row('tv', 'Qualità massima', 'Lume adatta la qualità alla connessione, fino al limite scelto.', `<select id="settings-quality" aria-label="Qualità massima"><option value="auto" ${p.quality === 'auto' ? 'selected' : ''}>Automatica</option>${['1080', '720', '480'].map(q => `<option value="${q}" ${p.quality === q ? 'selected' : ''}>Fino a ${q}p</option>`).join('')}</select>`)
      + row('volume', 'Avvia con audio disattivato', 'Il volume scelto viene ricordato per la prossima diretta.', `<button class="toggle" role="switch" aria-label="Avvia con audio disattivato" aria-checked="${p.muted}" data-setting-toggle="muted"></button>`)
      + `</div></section><section class="settings-section"><h2>Sempre al passo</h2><div class="settings-card">`
      + row('grid', 'Catalogo canali', escapeText(state.catalog.error || (state.catalog.loading ? 'Stiamo aggiornando la lista…' : `${state.channels.length} canali · ${state.catalog.updated_at ? 'aggiornato ' + date(state.catalog.updated_at) : 'catalogo incluso nell’app'}. Aggiornamento automatico ogni giorno.`)), `<button class="text-button" data-action="refresh-catalog" aria-label="Aggiorna catalogo canali" ${state.catalog.loading || state.catalog.next_retry_at > Date.now() ? 'disabled' : ''}>${icon('refresh')}Aggiorna</button>`)
      + row('guide', 'Guida TV', state.guide.next_retry_at > Date.now() ? `Il servizio richiede una pausa. Prossimo tentativo dalle ${time(state.guide.next_retry_at)}.` : state.guide.loading ? 'Stiamo aggiornando i programmi…' : state.guide.updated_at ? `Ultimo aggiornamento: ${date(state.guide.updated_at)}, ${time(state.guide.updated_at)}.` : 'La guida viene aggiornata automaticamente.', `<button class="text-button" data-action="refresh-guide" ${state.guide.loading || state.guide.next_retry_at > Date.now() ? 'disabled' : ''}>${icon('refresh')}Aggiorna</button>`)
      + row('clock', 'Canali visti di recente', 'La tua cronologia rimane su questo dispositivo.', `<button class="text-button" data-action="clear-recent" ${p.recent.length ? '' : 'disabled'}>Svuota cronologia</button>`)
      + row('keyboard', 'Scorciatoie da tastiera', 'Un modo ancora più veloce per sentirti a casa.', `<button class="text-button" data-action="shortcuts">Scoprile ${icon('arrow')}</button>`)
      + `</div><p class="settings-note">I promemoria ti avvisano mentre Lume è aperta. Canali e programmi possono cambiare in base alla disponibilità delle emittenti.</p></section><div class="about-lume"><img src="/assets/symbol.svg" alt=""><div><strong>Lume TV</strong><p>La TV, fatta semplice. · Versione ${escapeText(state.version)}</p></div></div></div>`;
  }

  function renderSearch() {
    const channels = filterChannels(state.channels, { query: state.query, programmes: state.guide.programmes });
    const words = searchable(state.query).split(' ').filter(Boolean);
    const programmes = state.channels.flatMap(channel => schedule(state.guide.programmes[channel.id])
      .filter(p => p.end > Date.now() && words.every(word => searchable(`${p.title} ${channel.name} ${p.description}`).includes(word)))
      .slice(0, 3).map(programme => ({ channel, programme }))).slice(0, 30);
    return heading('Un momento da trovare.', `Risultati per “${state.query}”`)
      + (channels.length ? `<div class="section-heading"><h2>Canali <span>${channels.length}</span></h2></div><div data-key="channels" class="channel-grid">${channels.map(channelCard).join('')}</div>` : '')
      + (programmes.length ? `<div class="section-heading"><h2>Programmi <span>${programmes.length}</span></h2></div><div data-key="programmes" class="discovery-list">${programmes.map(({ channel, programme: p }) => `<button class="discovery-item" data-programme="${channel.id}~${p.start}">${stationLogo(channel)}<span class="item-description"><strong>${escapeText(p.title)}</strong><small>${escapeText(channel.name)} · ${date(p.start)} · ${range(p)}</small></span><span class="remaining">${p.start <= Date.now() ? 'In onda' : time(p.start)}</span></button>`).join('')}</div>` : '')
      + (!channels.length && !programmes.length ? emptyState('search', 'Questo titolo si fa desiderare.', 'Prova il nome di un canale, di un programma o una parola diversa.') : '');
  }

  function render(resetScroll = false) {
    const active = doc.activeElement?.dataset?.focus;
    const scroll = resetScroll ? 0 : $('content-scroll').scrollTop;
    const guideScroll = $('guide-scroll');
    const guidePosition = guideScroll ? [guideScroll.scrollLeft, guideScroll.scrollTop] : null;
    doc.body.dataset.page = state.query ? 'search' : state.page;
    $('page-content').hidden = state.page === 'watch' && !state.query;
    if (!$('page-content').hidden) {
      ui.patch($('page-content'), state.query ? renderSearch() : ({ channels: renderChannels, favorites: () => renderChannels(true), discover: renderDiscovery, guide: renderGuide, settings: renderSettings }[state.page] || renderChannels)());
    }
    for (const button of doc.querySelectorAll('[data-route].nav-item')) {
      const selected = button.dataset.route === state.page && !state.query;
      button.classList.toggle('active', selected);
      if (selected) button.setAttribute('aria-current', 'page'); else button.removeAttribute('aria-current');
    }
    $('favorite-count').hidden = state.preferences.favorites.length === 0;
    $('favorite-count').textContent = state.preferences.favorites.length;
    const reminders = state.preferences.reminders.filter(r => r.start > Date.now());
    $('reminder-count').hidden = reminders.length === 0;
    $('reminder-count').textContent = reminders.length;
    $('watch-view').hidden = !state.current;
    renderWatch();
    $('catalog-status').textContent = state.guide.loading ? 'La tua guida sta arrivando…'
      : state.guide.error ? 'Canali disponibili · Guida da aggiornare'
        : state.guide.updated_at ? `Guida aggiornata alle ${time(state.guide.updated_at)}` : 'I tuoi canali, in un solo posto.';
    $('content-scroll').scrollTop = scroll;
    if (guidePosition && $('guide-scroll')) { $('guide-scroll').scrollLeft = guidePosition[0]; $('guide-scroll').scrollTop = guidePosition[1]; }
    if (active) doc.querySelector(`[data-focus="${global.CSS.escape(active)}"]`)?.focus({ preventScroll: true });
  }

  function navigate(page, preserveQuery = false) {
    if (!['channels', 'favorites', 'discover', 'guide', 'settings', 'watch'].includes(page)) return;
    if (page === 'watch' && !state.current) return;
    const changesDock = state.current && (doc.body.dataset.page === 'watch') !== (page === 'watch' && (!preserveQuery || !state.query));
    const before = changesDock ? ui.captureDock($('player-dock')) : null;
    ui.setOptions($('player-options'), false, true);
    $('player-settings').setAttribute('aria-expanded', 'false');
    if (state.page !== 'watch') state.previousPage = state.page;
    if (page !== state.page) { state.provider = 'all'; state.group = 'all'; }
    state.page = page;
    if (!preserveQuery) { state.query = ''; $('search').value = ''; }
    if (page !== 'favorites' && page !== 'channels') { state.provider = 'all'; state.group = 'all'; }
    render(true);
    if (changesDock) ui.moveDock($('player-dock'), before);
    doc.title = page === 'watch' ? `${state.channels.find(c => c.id === state.current)?.name || 'Diretta'} · Lume TV` : 'Lume TV';
  }

  function renderWatch() {
    const channel = state.channels.find(c => c.id === state.current) || state.session?.channel;
    $('sidebar-now').hidden = !channel;
    if (!channel) return;
    const now = currentProgramme(state.guide.programmes[channel.id]);
    const next = schedule(state.guide.programmes[channel.id]).find(p => p.start > Date.now());
    const favorite = state.preferences.favorites.includes(channel.id);
    ui.patch($('sidebar-now'), `<button data-route="watch">${stationLogo(channel)}<span><strong>${escapeText(channel.name)}</strong><small>Stai guardando</small></span><span class="equalizer"><i></i><i></i><i></i></span></button>`);
    $('player-channel-label').textContent = channel.name;
    $('control-channel').textContent = channel.name;
    $('mini-channel-name').textContent = channel.name;
    ui.patch($('watch-details'), `<div class="watch-heading">${stationLogo(channel)}<div><span class="watch-station-name">${escapeText(channel.name)}</span><small>Canale ${channel.number} · ${providerName(channel.provider)}</small></div><button class="icon-button" data-favorite="${channel.id}" aria-pressed="${favorite}" aria-label="${favorite ? 'Rimuovi dai' : 'Aggiungi ai'} preferiti" title="${favorite ? 'Rimuovi dai' : 'Aggiungi ai'} preferiti">${icon('star')}</button></div><div class="eyebrow">IN ONDA ADESSO</div><h1>${escapeText(now?.title || `La diretta di ${channel.name}`)}</h1>${now ? `<div class="programme-meta"><span>${range(now)}</span><span>${remainingLabel(now)}</span></div><p class="programme-description">${escapeText(now.description || '')}</p>` : '<p class="programme-description">La programmazione di questo canale non è al momento disponibile nella guida.</p>'}${next ? `<div class="watch-next"><span>A SEGUIRE</span><div><strong>${escapeText(next.title)}</strong><small>${range(next)}</small></div><button class="text-button" data-programme="${channel.id}~${next.start}" aria-label="Dettagli del prossimo programma">${icon('info')}</button></div>` : ''}`);
    const channels = state.watchFavorites ? state.channels.filter(c => state.preferences.favorites.includes(c.id)) : state.channels;
    const listScroll = $('watch-channel-list')?.scrollTop || 0;
    ui.patch($('watch-browser'), `<header><h2>Cambia canale</h2><div class="segmented"><button data-watch-filter="all" aria-pressed="${!state.watchFavorites}">Tutti</button><button data-watch-filter="favorites" aria-pressed="${state.watchFavorites}">Preferiti</button></div></header><div class="watch-channel-list" id="watch-channel-list">${channels.length ? channels.map(c => `<button class="watch-channel" data-play="${c.id}" aria-pressed="${state.current === c.id}" aria-label="Guarda ${escapeText(c.name)}"><span>${String(c.number).padStart(2, '0')}</span>${stationLogo(c)}<span class="watch-channel-description"><strong>${escapeText(c.name)}</strong><small>${escapeText(currentProgramme(state.guide.programmes[c.id])?.title || 'Guarda in diretta')}</small></span>${state.current === c.id ? '<span class="equalizer"><i></i><i></i><i></i></span>' : ''}</button>`).join('') : '<p class="notice">Aggiungi i tuoi canali preferiti con la stella.</p>'}</div>`);
    $('watch-channel-list').scrollTop = listScroll;
  }

  function showDialog(html) {
    ui.patch($('dialog-content'), html);
    ui.openDialog($('details-dialog'));
  }
  const closeDialogButton = () => `<button class="icon-button" data-action="close-dialog" aria-label="Chiudi">${icon('close')}</button>`;

  function programmeDetails(key) {
    const [id, start] = key.split('~');
    const channel = state.channels.find(c => c.id === id);
    const p = schedule(state.guide.programmes[id]).find(p => String(p.start) === start);
    if (!channel || !p) return;
    const reminder = state.preferences.reminders.some(r => r.channel_id === id && r.start === p.start);
    const future = p.start > Date.now();
    showDialog(`<div class="dialog-top">${stationLogo(channel)}<span><strong>${escapeText(channel.name)}</strong><small class="programme-meta">${date(p.start)}</small></span>${closeDialogButton()}</div><div class="eyebrow">${future ? 'A SEGUIRE' : p.end > Date.now() ? 'IN ONDA ADESSO' : 'GIÀ TRASMESSO'}</div><h2 id="dialog-title">${escapeText(p.title)}</h2><p class="programme-time">${range(p)}</p><p class="programme-description">${escapeText(p.description || 'La descrizione di questo programma non è disponibile nella guida.')}</p><div class="dialog-actions"><button class="button primary" data-play="${id}">${icon('play')}Guarda ${future || p.end <= Date.now() ? 'il canale' : 'ora'}</button>${future ? `<button class="button secondary" data-remind="${key}" aria-pressed="${reminder}">${icon(reminder ? 'check' : 'bell')}${reminder ? 'Promemoria attivo' : 'Ricordamelo'}</button>` : ''}</div>${future ? '<p class="settings-note">Ti avviseremo poco prima dell’inizio, mentre Lume è aperta.</p>' : ''}`);
  }

  function showReminders() {
    const list = state.preferences.reminders.filter(r => r.start > Date.now()).sort((a, b) => a.start - b.start);
    showDialog(`<div class="dialog-top"><div class="empty-symbol" style="margin:0;width:40px;height:40px;border-radius:12px">${icon('bell')}</div>${closeDialogButton()}</div><h2 id="dialog-title">Un momento da ricordare.</h2><p class="programme-meta">Gli avvisi arrivano mentre Lume è aperta.</p>${list.length ? `<div class="reminder-list">${list.map(r => `<div class="reminder-item"><div><strong>${escapeText(r.title)}</strong><small>${escapeText(state.channels.find(c => c.id === r.channel_id)?.name || '')} · ${date(r.start)}, ${time(r.start)}</small></div><button class="icon-button" data-remove-reminder="${r.channel_id}~${r.start}" aria-label="Rimuovi promemoria per ${escapeText(r.title)}">${icon('close')}</button></div>`).join('')}</div>` : '<p class="programme-description">Hai visto qualcosa di interessante nella guida? Apri il programma e premi “Ricordamelo”. Lo ritroverai qui.</p><button class="button secondary" data-route="guide">Esplora la guida</button>'}`);
  }

  function showShortcuts() {
    const modifier = /Mac/i.test(global.navigator.platform) ? '⌘' : 'Ctrl';
    showDialog(`<div class="dialog-top"><span class="eyebrow" style="margin:0">TUTTO A PORTATA DI MANO</span>${closeDialogButton()}</div><h2 id="dialog-title">Il piacere di fare prima.</h2><div class="shortcut-list">${[
      ['Cerca un canale o programma', `${modifier} K`], ['Canali · Cosa guardare · Guida · Preferiti', `${modifier} 1 / 2 / 3 / 4`],
      ['Play / Pausa', 'Spazio'], ['Attiva / Disattiva audio', 'M'], ['Schermo intero', 'F'], ['Canale precedente / successivo', '↑ / ↓'], ['Torna al catalogo durante la diretta', 'Esc'],
    ].map(([label, key]) => `<div><span>${label}</span><kbd>${key}</kbd></div>`).join('')}</div>`);
  }

  function setPhase(phase, title, description = '') {
    state.phase = phase;
    $('player-overlay').hidden = phase === 'playing' || phase === 'paused';
    $('player-overlay').classList.toggle('error', ['error', 'interaction'].includes(phase));
    $('player-status').textContent = title || '';
    $('player-status-detail').textContent = description;
    $('player-retry').hidden = phase !== 'error';
    $('player-start').hidden = phase !== 'interaction';
    $('player-dock').classList.remove('controls-hidden');
  }

  function emit(event, session = state.session) {
    if (!session) return;
    apiFetch('/api/event', { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Lume-Playback-Id': session.playback_id }, body: JSON.stringify(event) }).catch(() => {});
  }

  function videoSample() {
    const video = $('video');
    const quality = video.getVideoPlaybackQuality?.();
    return { kind: 'sample', current_time: video.currentTime || 0, video_frames: quality?.totalVideoFrames || video.webkitDecodedFrameCount || 0,
      dropped_frames: quality?.droppedVideoFrames || 0, width: video.videoWidth, height: video.videoHeight, ready_state: video.readyState,
      muted: video.muted, paused: video.paused, seeking: video.seeking,
      ...(typeof video.webkitAudioDecodedByteCount === 'number' ? { audio_decoded_bytes: video.webkitAudioDecodedByteCount, audio_decode_metric_available: true } : {}) };
  }

  async function ensurePlayer() {
    if (playerPromise) return playerPromise;
    playerPromise = (async () => {
      const shaka = global.shaka;
      if (!shaka) throw new Error('player_missing');
      if (shaka.log?.setLevel) shaka.log.setLevel(shaka.log.Level.NONE);
      shaka.polyfill.installAll();
      shaka.net.NetworkingEngine.registerScheme('lume', global.LumePlayback.makeRelayPlugin(shaka, (path, options) => apiFetch(path, {
        ...options, headers: { ...(options.headers || {}), 'X-Lume-Playback-Id': state.session?.playback_id || '' },
      }), 'media'));
      shaka.net.NetworkingEngine.registerScheme('lume-license', global.LumePlayback.makeRelayPlugin(shaka, (path, options) => apiFetch(path, {
        ...options, headers: { ...(options.headers || {}), 'X-Lume-Playback-Id': state.session?.playback_id || '' },
      }), 'license'));
      player = new shaka.Player();
      await player.attach($('video'));
      player.getNetworkingEngine().registerRequestFilter((_, request) => {
        request.uris = request.uris.map(uri => /^https?:\/\//i.test(uri) ? 'lume:' + uri : uri);
      });
      player.configure({ preferredAudioLanguage: 'it', streaming: { bufferingGoal: 15, rebufferingGoal: 2 },
        manifest: { retryParameters: { maxAttempts: 3, baseDelay: 800, backoffFactor: 2 } } });
      player.addEventListener('error', event => {
        if (state.pending || [7000, 7001].includes(event.detail?.code)) return;
        emit({ kind: 'error', ...global.LumePlayback.sanitizedError(event.detail) });
        setPhase('error', 'La diretta si è interrotta.', 'Controlla la connessione e riprova. Puoi anche scegliere un altro canale.');
      });
      player.addEventListener('buffering', event => {
        if (event.buffering && state.phase === 'playing' && !state.pending) setPhase('buffering', 'Riprendiamo il filo.', 'Stiamo recuperando la diretta…');
        else if (!event.buffering && state.phase === 'buffering') setPhase('playing');
      });
      player.addEventListener('keystatuschanged', () => emit({ kind: 'key_statuses', key_statuses: global.LumePlayback.safeKeyStatuses(player.getKeyStatuses()) }));
      player.addEventListener('trackschanged', updateTracks);
      player.addEventListener('variantchanged', updateTracks);
      player.addEventListener('adaptation', updateTracks);
      return player;
    })().catch(error => { playerPromise = null; throw error; });
    return playerPromise;
  }

  function applyQuality() {
    if (!player) return;
    const maximum = state.preferences.quality === 'auto' ? Infinity : Number(state.preferences.quality);
    player.configure({ abr: { enabled: true }, restrictions: { maxHeight: maximum } });
  }

  function updateTracks() {
    if (!player) return;
    const tracks = player.getVariantTracks();
    const active = tracks.find(t => t.active);
    $('video-quality').textContent = active?.height ? `${active.height}p` : '';
    const languages = Array.from(new Set(tracks.map(t => t.language).filter(Boolean)));
    ui.patch($('audio-track'), languages.map(language => {
      let label = language;
      try { label = new Intl.DisplayNames(['it'], { type: 'language' }).of(language); } catch (_) { /* use language code */ }
      return `<option value="${escapeText(language)}" ${language === active?.language ? 'selected' : ''}>${escapeText(label)}</option>`;
    }).join('') || '<option>Predefinito</option>');
    const textTracks = player.getTextTracks();
    const activeText = textTracks.find(t => t.active);
    ui.patch($('text-track'), '<option value="off">Disattivati</option>' + textTracks.map(t => `<option value="${t.id}" ${player.isTextTrackVisible() && t.id === activeText?.id ? 'selected' : ''}>${escapeText(t.label || t.language || 'Sottotitoli')}</option>`).join(''));
  }

  async function loadChannel(id) {
    const channel = state.channels.find(c => c.id === id);
    if (!channel) return;
    if (id === state.current && ['playing', 'paused', 'buffering'].includes(state.phase)) { navigate('watch'); return; }
    const generation = ++state.generation;
    let pendingSession;
    loadController?.abort();
    loadController = new AbortController();
    state.pending = id;
    state.current = id;
    state.playedId = null;
    $('player-dock').classList.remove('has-picture');
    $('video').pause();
    navigate('watch');
    setPhase('loading', `Sintonizzazione su ${channel.name}…`, 'Ancora un istante, il tuo canale sta arrivando.');
    try {
      const signal = loadController.signal;
      const session = await bounded(api('/api/play', { channel_id: id }, { signal }), 60_000, signal);
      if (generation !== state.generation) return;
      pendingSession = session;
      await bounded(ensurePlayer(), 10_000, signal);
      if (generation !== state.generation) return;
      await bounded(player.unload(), 10_000, signal);
      if (generation !== state.generation) return;
      state.session = session;
      let capabilities = await bounded(global.LumePlayback.probeCapabilities($('video')), 12_000, signal);
      if (generation !== state.generation) return;
      emit({ kind: 'capabilities', ...capabilities });
      if (!capabilities.avc_mse_supported || !capabilities.aac_mse_supported) throw { code: 'codec_unavailable' };
      if (session.stream.drm && !capabilities.widevine_available) {
        const setup = await bounded(global.LumePlayback.waitForWidevine(capabilities, {
          timeoutSeconds: drmSetupSeconds, probe: () => bounded(global.LumePlayback.probeCapabilities($('video')), 12_000, signal),
          isCancelled: () => generation !== state.generation,
          onWaiting: () => { setPhase('loading', 'Prepariamo i canali protetti.', 'Solo al primo avvio può servire qualche minuto. Lascia Lume aperta e verifica la connessione.'); emit({ kind: 'drm_setup', status: 'waiting' }); },
        }), (drmSetupSeconds + 15) * 1000, signal);
        if (generation !== state.generation) return;
        capabilities = setup.capabilities;
        emit({ kind: 'capabilities', ...capabilities });
        emit({ kind: 'drm_setup', status: setup.status, elapsed_seconds: setup.elapsed_seconds });
        if (!capabilities.widevine_available) throw { code: 'widevine_unavailable' };
      }
      player.configure({ drm: { servers: session.stream.drm ? { 'com.widevine.alpha': 'lume-license:license' } : {} } });
      applyQuality();
      await bounded(player.load(session.stream.url, undefined, session.stream.mime_type || undefined), 45_000, signal);
      if (generation !== state.generation) return;
      emit({ kind: 'loaded', drm_configured: !!session.stream.drm });
      state.pending = null;
      applyPreferences();
      await resumeVideo();
      updateTracks();
      global.clearInterval(sampleTimer);
      sampleTimer = global.setInterval(() => { if (state.session && $('video').readyState >= 2) emit(videoSample()); }, 2000);
    } catch (error) {
      if (generation !== state.generation || error.name === 'AbortError') return;
      ++state.generation;
      loadController?.abort();
      player?.unload().catch(() => {});
      state.pending = null;
      emit({ kind: 'error', ...global.LumePlayback.sanitizedError(error) }, pendingSession);
      const message = error.code === 'widevine_unavailable'
        ? ['I canali protetti non sono ancora pronti.', 'Verifica la connessione, poi riprova o riapri Lume.']
        : error.code === 'codec_unavailable'
          ? ['Questo formato non è disponibile.', 'Il motore video di questa versione non supporta il formato del canale.']
          : ['Questo canale si fa attendere.', 'La sorgente non è al momento disponibile. Riprova tra poco o scegli un altro canale.'];
      setPhase('error', ...message);
    }
  }

  async function resumeVideo() {
    try { await $('video').play(); }
    catch (_) { setPhase('interaction', 'Il tuo canale è pronto.', 'Premi Avvia per iniziare la visione con audio.'); }
  }

  async function stopPlayback() {
    ui.stopDock($('player-dock'));
    const generation = ++state.generation;
    const playbackId = state.session?.playback_id;
    loadController?.abort();
    state.pending = null;
    global.clearInterval(sampleTimer);
    api('/api/stop', { playback_id: playbackId || null }).catch(() => {});
    $('video').pause();
    if (player) await player.unload().catch(() => {});
    if (generation !== state.generation) return;
    state.current = null; state.session = null; state.phase = 'idle';
    if (state.page === 'watch') navigate(state.previousPage === 'watch' ? 'channels' : state.previousPage); else render();
    doc.title = 'Lume TV';
  }

  async function refreshCatalog(force = false) {
    global.clearTimeout(catalogPoll);
    try {
      const snapshot = force ? await api('/api/catalog/refresh', {}) : await api('/api/catalog');
      const { channels, ...status } = snapshot;
      const changed = status.revision !== state.catalog.revision;
      state.catalog = status;
      if (changed) {
        state.channels = channels;
        if (!channels.some(c => matchesProvider(c, state.provider))) state.provider = 'all';
        if (state.group !== 'all' && !channels.some(c => c.group === state.group)) state.group = 'all';
        render();
        await refreshGuide();
      } else if (state.page === 'settings') render();
      catalogPoll = global.setTimeout(() => refreshCatalog(), status.loading ? 1800 : 300_000);
      if (force && !status.loading) toast(status.error || 'Il catalogo è aggiornato.');
    } catch (_) {
      state.catalog.loading = false;
      state.catalog.error = 'Non riesco ad aggiornare la lista. I canali salvati restano disponibili.';
      catalogPoll = global.setTimeout(() => refreshCatalog(), 300_000);
      if (state.page === 'settings') render();
    }
  }

  async function refreshGuide(force = false) {
    global.clearTimeout(guidePoll);
    try {
      state.guide = force ? await api('/api/guide/refresh', {}) : await api('/api/guide');
      render();
      guidePoll = global.setTimeout(() => refreshGuide(false), state.guide.loading ? 1800 : 300_000);
      if (!state.guide.loading && force) toast(state.guide.error || 'La guida è aggiornata.');
    } catch (_) {
      state.guide.loading = false;
      state.guide.error = 'Non riesco ad aggiornare la guida. I canali restano disponibili.';
      guidePoll = global.setTimeout(() => refreshGuide(false), 300_000);
      render();
    }
  }

  function updateVolumeIcon() {
    const video = $('video');
    $('mute').innerHTML = icon(video.muted || video.volume === 0 ? 'muted' : 'volume');
    $('mute').setAttribute('aria-label', video.muted ? 'Attiva audio' : 'Disattiva audio');
  }

  async function fullscreen() {
    try { if (doc.fullscreenElement) await doc.exitFullscreen(); else await $('player-dock').requestFullscreen(); }
    catch (_) { toast('Lo schermo intero non è disponibile in questa finestra.'); }
  }

  function zap(direction) {
    const channels = state.preferences.zap_favorites_only && state.preferences.favorites.length
      ? state.channels.filter(c => state.preferences.favorites.includes(c.id)) : state.channels;
    if (!channels.length) return;
    const index = channels.findIndex(c => c.id === state.current);
    loadChannel(channels[(Math.max(0, index) + direction + channels.length) % channels.length].id);
  }

  doc.addEventListener('error', event => {
    if (event.target instanceof global.HTMLImageElement && event.target.closest('.station-logo')) event.target.hidden = true;
  }, true);
  doc.addEventListener('load', event => {
    if (event.target instanceof global.HTMLImageElement && event.target.closest('.station-logo')) event.target.setAttribute('data-loaded', '');
  }, true);
  doc.addEventListener('click', async event => {
    const button = event.target.closest('button, a.brand');
    if (!button || button.disabled) return;
    if (button.matches('a.brand')) { event.preventDefault(); navigate('channels'); return; }
    if (button.dataset.route) { ui.closeDialog($('details-dialog'), true); navigate(button.dataset.route); }
    if (button.dataset.play) { ui.closeDialog($('details-dialog'), true); loadChannel(button.dataset.play); }
    if (button.dataset.favorite) {
      const id = button.dataset.favorite;
      const card = button.closest('.channel-card');
      const neighbour = card?.nextElementSibling?.querySelector('[data-favorite]')
        || card?.previousElementSibling?.querySelector('[data-favorite]');
      const restoreFavoriteFocus = doc.documentElement.dataset.input === 'keyboard' && doc.activeElement === button;
      await savePreferences({ toggle_favorite: id }, state.preferences.favorites.includes(id) ? 'Rimosso dai preferiti.' : 'Aggiunto ai tuoi preferiti.');
      if (button.isConnected) ui.confirmFavorite(button);
      else if (restoreFavoriteFocus && doc.activeElement === doc.body && state.page === 'favorites') {
        (neighbour?.isConnected ? neighbour : doc.querySelector('.empty-state button'))?.focus({ preventScroll: true });
      }
    }
    if (button.dataset.provider) { state.provider = button.dataset.provider; render(); }
    if (button.dataset.period) { state.period = button.dataset.period; state.discoveryKey = null; render(); }
    if (button.dataset.discover) { state.discoveryKey = button.dataset.discover; render(); }
    if (button.dataset.programme) programmeDetails(button.dataset.programme);
    if (button.dataset.theme) await savePreferences({ theme: button.dataset.theme });
    if (button.dataset.settingToggle) {
      const key = button.dataset.settingToggle;
      await savePreferences({ [key]: !state.preferences[key] });
    }
    if (button.dataset.guideFavorites) { state.favoritesOnly = button.dataset.guideFavorites === 'true'; render(); }
    if (button.dataset.guideTime) {
      const start = new Date();
      if (button.dataset.guideTime === 'tonight') start.setHours(20, 0, 0, 0);
      state.guideStart = Math.floor(start.getTime() / 1_800_000) * 1_800_000; render();
    }
    if (button.dataset.guideShift) {
      state.guideStart = Math.min(Date.now() + 44 * 3_600_000, Math.max(Date.now() - 6 * 3_600_000, state.guideStart + Number(button.dataset.guideShift) * 4 * 3_600_000));
      state.guideStart = Math.floor(state.guideStart / 1_800_000) * 1_800_000; render();
    }
    if (button.dataset.watchFilter) { state.watchFavorites = button.dataset.watchFilter === 'favorites'; renderWatch(); }
    if (button.dataset.remind || button.dataset.removeReminder) {
      const key = button.dataset.remind || button.dataset.removeReminder;
      const [id, start] = key.split('~');
      const p = state.guide.programmes[id]?.find(p => String(p.start) === start)
        || state.preferences.reminders.find(r => r.channel_id === id && String(r.start) === start);
      if (p) {
        await savePreferences({ toggle_reminder: { channel_id: id, start: Number(start), title: p.title } }, 'Promemoria aggiornato.');
        if (button.dataset.remind) programmeDetails(key); else showReminders();
      }
    }
    switch (button.dataset.action) {
      case 'close-dialog': ui.closeDialog($('details-dialog')); break;
      case 'reset-filters': state.provider = 'all'; state.group = 'all'; state.query = ''; $('search').value = ''; navigate('channels'); break;
      case 'discover-favorites': state.favoritesOnly = !state.favoritesOnly; render(); break;
      case 'refresh-catalog': refreshCatalog(true); break;
      case 'refresh-guide': refreshGuide(true); break;
      case 'clear-recent': await savePreferences({ clear_recent: true }, 'Cronologia svuotata.'); break;
      case 'shortcuts': showShortcuts(); break;
    }
  });

  $('search').addEventListener('input', event => { state.query = event.target.value; render(); });
  doc.addEventListener('change', event => {
    if (event.target.id === 'group-filter') { state.group = event.target.value; render(); }
    if (['settings-quality', 'playback-quality'].includes(event.target.id)) savePreferences({ quality: event.target.value }, '', event.target.id === 'settings-quality');
    if (event.target.id === 'audio-track') player?.selectAudioLanguage(event.target.value);
    if (event.target.id === 'text-track' && player) {
      const track = player.getTextTracks().find(t => String(t.id) === event.target.value);
      if (track) player.selectTextTrack(track);
      player.setTextTrackVisibility(!!track);
    }
  });
  $('details-dialog').addEventListener('click', event => { if (event.target === $('details-dialog')) ui.closeDialog($('details-dialog')); });
  $('details-dialog').addEventListener('cancel', event => { event.preventDefault(); ui.closeDialog($('details-dialog')); });
  $('shortcuts-button').addEventListener('click', showShortcuts);
  $('reminders-button').addEventListener('click', showReminders);
  $('mini-expand').addEventListener('click', () => navigate('watch'));
  $('mini-close').addEventListener('click', stopPlayback);
  $('player-close').addEventListener('click', stopPlayback);
  $('player-retry').addEventListener('click', () => state.current && loadChannel(state.current));
  $('player-start').addEventListener('click', resumeVideo);
  $('play-pause').addEventListener('click', () => { if ($('video').paused) resumeVideo(); else $('video').pause(); });
  $('mute').addEventListener('click', () => { $('video').muted = !$('video').muted; state.preferences.muted = $('video').muted; updateVolumeIcon(); savePreferences({ muted: $('video').muted }, '', false); });
  $('volume').addEventListener('input', event => {
    $('video').volume = Number(event.target.value); $('video').muted = false;
    state.preferences.volume = $('video').volume; state.preferences.muted = false; updateVolumeIcon();
    global.clearTimeout(volumeSaveTimer);
    volumeSaveTimer = global.setTimeout(() => savePreferences({ volume: $('video').volume, muted: $('video').muted }, '', false), 250);
  });
  $('player-settings').addEventListener('click', () => {
    const expanded = $('player-settings').getAttribute('aria-expanded') !== 'true';
    $('player-settings').setAttribute('aria-expanded', String(expanded));
    ui.setOptions($('player-options'), expanded);
    if (expanded) updateTracks();
  });
  $('fullscreen').addEventListener('click', fullscreen);
  $('video').addEventListener('dblclick', fullscreen);
  $('go-live').addEventListener('click', () => { try { $('video').currentTime = player.seekRange().end - .3; resumeVideo(); } catch (_) { /* not seekable yet */ } });
  $('picture-in-picture').hidden = !doc.pictureInPictureEnabled;
  $('picture-in-picture').addEventListener('click', async () => { try { if (doc.pictureInPictureElement) await doc.exitPictureInPicture(); else await $('video').requestPictureInPicture(); } catch (_) { toast('Picture in picture non è disponibile per questa diretta.'); } });
  $('player-dock').addEventListener('mousemove', () => {
    $('player-dock').classList.remove('controls-hidden');
    global.clearTimeout(hideControlsTimer);
    hideControlsTimer = global.setTimeout(() => { if (state.phase === 'playing' && $('player-options').hidden) $('player-dock').classList.add('controls-hidden'); }, 3000);
  });
  $('video').addEventListener('playing', () => {
    if (state.pending || !state.session) return;
    setPhase('playing'); $('player-dock').classList.add('has-picture');
    $('play-pause').innerHTML = icon('pause'); $('play-pause').setAttribute('aria-label', 'Pausa');
    emit({ ...videoSample(), kind: 'playing' });
    if (state.playedId !== state.session.playback_id) {
      state.playedId = state.session.playback_id;
      api('/api/played', { playback_id: state.session.playback_id }).then(preferences => { state.preferences = preferences; renderWatch(); }).catch(() => {});
    }
  });
  $('video').addEventListener('pause', () => {
    $('play-pause').innerHTML = icon('play'); $('play-pause').setAttribute('aria-label', 'Riprendi');
    if (!state.pending && state.phase === 'playing') setPhase('paused');
  });
  $('video').addEventListener('encrypted', () => emit({ kind: 'encrypted', encrypted: true, encrypted_events: 1 }));
  $('video').addEventListener('volumechange', updateVolumeIcon);
  global.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => applyPreferences());
  global.addEventListener('online', () => { if (state.phase === 'error' && state.current) loadChannel(state.current); if (state.guide.error) refreshGuide(); refreshCatalog(); });
  doc.addEventListener('keydown', event => {
    const modifier = event.metaKey || event.ctrlKey;
    if (modifier && event.key.toLowerCase() === 'k') { event.preventDefault(); $('search').focus(); $('search').select(); return; }
    if (modifier && ['1', '2', '3', '4'].includes(event.key)) { event.preventDefault(); navigate(['channels', 'discover', 'guide', 'favorites'][Number(event.key) - 1]); return; }
    if ($('details-dialog').open) return;
    if (event.key === 'Escape') {
      if (doc.fullscreenElement) { event.preventDefault(); doc.exitFullscreen().catch(() => {}); }
      else if (state.query) { state.query = ''; $('search').value = ''; render(); $('search').blur(); }
      else if (state.page === 'watch' && !doc.fullscreenElement) navigate(state.previousPage === 'watch' ? 'channels' : state.previousPage);
      return;
    }
    if (event.target.closest('input,select,textarea') || modifier || state.page !== 'watch' ||
        (event.key === ' ' && event.target.closest('button'))) return;
    if (event.key === ' ') { event.preventDefault(); if ($('video').paused) resumeVideo(); else $('video').pause(); }
    if (event.key.toLowerCase() === 'm') $('mute').click();
    if (event.key.toLowerCase() === 'f') fullscreen();
    if (event.key === 'ArrowDown' || event.key === 'PageDown') { event.preventDefault(); zap(1); }
    if (event.key === 'ArrowUp' || event.key === 'PageUp') { event.preventDefault(); zap(-1); }
  });

  function tick() {
    $('clock').textContent = time(Date.now());
    $('clock').dateTime = new Date().toISOString();
    for (const reminder of state.preferences.reminders) {
      const key = `${reminder.channel_id}~${reminder.start}`;
      if (!reminderShown.has(key) && reminder.start - Date.now() <= 60_000 && reminder.start >= Date.now() - 120_000) {
        reminderShown.add(key);
        toast(`Sta per iniziare: ${reminder.title}`, { label: 'Guarda', run: () => loadChannel(reminder.channel_id) });
      }
    }
  }

  async function bootstrap() {
    try {
      const fragment = decodeURIComponent(global.location.hash.slice(1));
      if (/^[A-Za-z0-9_-]{16,256}$/.test(fragment)) { token = fragment; global.sessionStorage.setItem('lume-session', token); }
      else token = global.sessionStorage.getItem('lume-session') || '';
      global.history.replaceState(null, '', global.location.pathname + global.location.search);
      if (!token) throw new Error('session_missing');
      const initial = await api('/api/bootstrap');
      state.channels = initial.channels; state.preferences = initial.preferences;
      if (initial.catalog) state.catalog = initial.catalog;
      state.version = initial.version; state.os = initial.os;
      $('search-shortcut').textContent = /Mac/i.test(global.navigator.platform) ? '⌘ K' : 'Ctrl K';
      applyPreferences(); render(); tick();
      await Promise.all([refreshGuide(), refreshCatalog()]);
      ui.introduceBrand(doc.querySelector('.hero-mark'));
      global.setInterval(tick, 15_000);
      global.setInterval(() => { if (!$('details-dialog').open) render(); }, 60_000);
    } catch (_) {
      $('page-content').innerHTML = emptyState('wifi', 'Ritroviamo la connessione con Lume.', 'Chiudi e riapri l’app per ripartire con una nuova sessione. I tuoi preferiti restano al sicuro.');
    }
  }
  bootstrap();
})(typeof window === 'undefined' ? globalThis : window);
