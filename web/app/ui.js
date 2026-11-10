/* SPDX-License-Identifier: GPL-3.0-or-later */
(function (root, factory) {
  const ui = factory(root);
  if (typeof module === 'object' && module.exports) module.exports = ui;
  else root.LumeUI = ui;
})(typeof window === 'undefined' ? globalThis : window, function (global) {
  'use strict';

  const OUT = 'cubic-bezier(0.23, 1, 0.32, 1)';
  const DRAWER = 'cubic-bezier(0.32, 0.72, 0, 1)';
  const surfaces = new Map();
  const docks = new Map();
  const markup = new WeakMap();
  let modality = 'pointer';
  let initialized = false;
  let brandIntroduced = false;
  let brandAnimation = null;
  const reduced = () => !!global.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
  const canMove = () => !reduced() && modality !== 'keyboard' && !global.document?.hidden;
  const token = (name, fallback) => global.document?.documentElement && global.getComputedStyle
    ? global.getComputedStyle(global.document.documentElement).getPropertyValue(name).trim() || fallback : fallback;
  const milliseconds = (name, fallback) => {
    const value = token(name, `${fallback}ms`);
    const number = Number.parseFloat(value);
    return Number.isFinite(number) ? number * (value.endsWith('ms') ? 1 : 1000) : fallback;
  };

  function key(node) {
    if (node.nodeType !== 1) return null;
    const action = ['data-play', 'data-provider', 'data-period', 'data-favorite', 'data-programme', 'data-theme', 'data-watch-filter', 'data-remind', 'data-remove-reminder']
      .find(name => node.hasAttribute(name));
    const value = node.id || node.getAttribute('data-key') || node.getAttribute('data-focus') ||
      (action ? `${action}:${node.getAttribute(action)}` : null);
    return value ? `${node.namespaceURI}:${node.tagName}:${value}`
      : node.tagName === 'OPTION' ? `option:${node.value}` : null;
  }
  const compatible = (a, b) => a.nodeType === b.nodeType &&
    (a.nodeType !== 1 || (a.tagName === b.tagName && a.namespaceURI === b.namespaceURI));

  function patchNode(current, next) {
    if (current.nodeType !== 1) {
      if (current.nodeValue !== next.nodeValue) current.nodeValue = next.nodeValue;
      return;
    }
    const sameImage = current.tagName === 'IMG' && current.getAttribute('src') === next.getAttribute('src');
    for (const attribute of Array.from(current.attributes)) {
      if (!next.hasAttribute(attribute.name) && !(sameImage && ['hidden', 'data-loaded'].includes(attribute.name))) {
        current.removeAttribute(attribute.name);
      }
    }
    for (const attribute of Array.from(next.attributes)) {
      if (current.getAttribute(attribute.name) !== attribute.value) {
        current.setAttribute(attribute.name, attribute.value);
      }
    }
    patchChildren(current, next);
    if (current.tagName === 'SELECT' && current.ownerDocument.activeElement !== current && current.value !== next.value) {
      current.value = next.value;
    }
  }

  function patchChildren(parent, next) {
    const existing = Array.from(parent.childNodes);
    const originals = new Set(existing);
    const keyed = new Map(existing.filter(node => key(node)).map(node => [key(node), node]));
    const retained = new Set();
    let cursor = parent.firstChild;
    for (const desired of Array.from(next.childNodes)) {
      const desiredKey = key(desired);
      let node = desiredKey ? keyed.get(desiredKey) : cursor;
      if (!node || retained.has(node) || (!desiredKey && key(node)) || !compatible(node, desired)) node = null;
      if (!node) node = desired.cloneNode(true);
      if (node !== cursor) parent.insertBefore(node, cursor);
      if (originals.has(node)) patchNode(node, desired);
      retained.add(node);
      cursor = node.nextSibling;
    }
    for (const node of existing) if (!retained.has(node)) node.remove();
  }

  // Inert templates keep parsing separate from the live tree. Stable keys retain
  // focus, scroll, decoded logos and in-flight feedback during data refreshes.
  function patch(parent, html) {
    if (markup.get(parent) === html) return;
    const template = parent.ownerDocument.createElement('template');
    template.innerHTML = html;
    patchChildren(parent, template.content);
    markup.set(parent, html);
  }

  function initialize(document) {
    if (initialized) return;
    initialized = true;
    const setModality = value => {
      if (modality === value) return;
      modality = value;
      document.documentElement?.setAttribute('data-input', value);
    };
    document.documentElement?.setAttribute('data-input', modality);
    document.addEventListener('pointerdown', () => setModality('pointer'), true);
    document.addEventListener('keydown', () => setModality('keyboard'), true);
    const settle = () => {
      for (const record of Array.from(surfaces.values())) record.finish();
      for (const dock of Array.from(docks.keys())) stopDock(dock);
      brandAnimation?.cancel();
    };
    document.addEventListener('visibilitychange', () => { if (document.hidden) settle(); });
    global.addEventListener('resize', () => { for (const dock of Array.from(docks.keys())) stopDock(dock); });
    document.addEventListener('fullscreenchange', () => { for (const dock of Array.from(docks.keys())) stopDock(dock); });
    global.matchMedia?.('(prefers-reduced-motion: reduce)').addEventListener('change', settle);
  }

  function surface(element, visible, { dialog = false, immediate = false } = {}) {
    const previous = surfaces.get(element);
    const wasVisible = dialog ? element.open : !element.hidden;
    if (visible && wasVisible && !previous) return;
    const computed = wasVisible ? global.getComputedStyle(element) : null;
    const from = previous && computed ? { opacity: computed.opacity, transform: computed.transform } : null;
    if (previous) { surfaces.delete(element); previous.animation.cancel(); }
    const closingTransform = dialog ? 'scale(0.97)' : 'translateY(4px) scale(0.97)';
    if (visible) {
      if (dialog) { if (!element.open) element.showModal(); }
      else element.hidden = false;
    }
    element.toggleAttribute('data-closing', !visible);
    element.toggleAttribute('data-instant', immediate || modality === 'keyboard');
    element.inert = !visible;
    let record;
    const finish = () => {
      if (record && surfaces.get(element) !== record) return;
      surfaces.delete(element);
      if (!visible) {
        if (dialog) { if (element.open) element.close(); }
        else element.hidden = true;
      }
      element.inert = false;
      element.removeAttribute('data-closing');
      element.style.removeProperty('will-change');
      record?.animation.cancel();
    };
    if (immediate || modality === 'keyboard' || !element.animate || global.document?.hidden || (!visible && !wasVisible)) {
      finish(); return;
    }
    const duration = reduced() ? 100 : dialog
      ? milliseconds(visible ? '--motion-dialog' : '--motion-exit', visible ? 220 : 140)
      : milliseconds(visible ? '--motion-popover' : '--motion-popover-exit', visible ? 160 : 120);
    const transform = reduced() ? 'none' : closingTransform;
    element.style.willChange = reduced() ? 'opacity' : 'transform, opacity';
    const animation = element.animate([
      from || { opacity: visible ? 0 : 1, transform: visible ? transform : 'none' },
      { opacity: visible ? 1 : 0, transform: visible ? 'none' : transform },
    ], { duration, easing: token('--ease-out', OUT), fill: 'both' });
    record = { animation, finish };
    surfaces.set(element, record);
    animation.finished.then(finish).catch(() => {});
  }

  const openDialog = dialog => surface(dialog, true, { dialog: true });
  const closeDialog = (dialog, immediate = false) => surface(dialog, false, { dialog: true, immediate });
  const setOptions = (element, visible, immediate = false) => surface(element, visible, { immediate });

  function confirmFavorite(button) {
    const svg = button?.querySelector('svg');
    if (!canMove() || !svg?.animate) return;
    for (const animation of svg.getAnimations()) animation.cancel();
    svg.animate([{ transform: 'scale(0.9)' }, { transform: 'scale(1)' }], {
      duration: milliseconds('--motion-press', 140), easing: token('--ease-out', OUT),
    });
  }

  function stopDock(dock) {
    const record = docks.get(dock);
    if (!record) return;
    docks.delete(dock);
    for (const animation of record.animations) animation.cancel();
    for (const element of [dock, dock.querySelector('video')]) element?.style.removeProperty('will-change');
    dock.removeAttribute('data-dock-moving');
  }

  const rect = element => {
    const r = element.getBoundingClientRect();
    return { x: r.x, y: r.y, width: r.width, height: r.height };
  };
  function captureDock(dock) {
    const video = dock.querySelector('video');
    const before = { dock: rect(dock), video: rect(video) };
    stopDock(dock);
    return before.dock.width > 0 && before.dock.height > 0 ? before : null;
  }

  // Counter-transform the live video so the frame can change proportions for
  // mini controls without stretching the picture. No bitmap/video snapshots.
  function dockFrames(before, after) {
    if (![before.dock, before.video, after.dock, after.video].every(r =>
      [r.x, r.y, r.width, r.height].every(Number.isFinite) && r.width > 0 && r.height > 0)) return null;
    const mix = (a, b, t) => a + (b - a) * t;
    const frames = { dock: [], video: [] };
    for (let i = 0; i <= 16; i++) {
      const t = i / 16;
      const sx = mix(before.dock.width / after.dock.width, 1, t);
      const sy = mix(before.dock.height / after.dock.height, 1, t);
      const dx = mix(before.dock.x - after.dock.x, 0, t);
      const dy = mix(before.dock.y - after.dock.y, 0, t);
      const vx = (mix(before.video.x, after.video.x, t) - after.dock.x - dx) / sx - (after.video.x - after.dock.x);
      const vy = (mix(before.video.y, after.video.y, t) - after.dock.y - dy) / sy - (after.video.y - after.dock.y);
      const vsx = mix(before.video.width, after.video.width, t) / (after.video.width * sx);
      const vsy = mix(before.video.height, after.video.height, t) / (after.video.height * sy);
      frames.dock.push({ offset: t, transform: `translate(${dx}px, ${dy}px) scale(${sx}, ${sy})` });
      frames.video.push({ offset: t, transform: `translate(${vx}px, ${vy}px) scale(${vsx}, ${vsy})` });
    }
    return frames;
  }

  function moveDock(dock, before) {
    const video = dock.querySelector('video');
    if (!before || !canMove() || !dock.animate || global.document.fullscreenElement) return;
    const after = { dock: rect(dock), video: rect(video) };
    const frames = dockFrames(before, after);
    if (!frames || (Math.abs(before.dock.width - after.dock.width) < 1 &&
      Math.abs(before.dock.x - after.dock.x) < 1 && Math.abs(before.dock.y - after.dock.y) < 1)) return;
    dock.setAttribute('data-dock-moving', '');
    const animations = [];
    const options = { duration: milliseconds('--motion-player', 280), easing: token('--ease-drawer', DRAWER), fill: 'both' };
    for (const [element, key] of [[dock, 'dock'], [video, 'video']]) {
      element.style.willChange = 'transform';
      animations.push(element.animate(frames[key], options));
    }
    const started = global.document.timeline?.currentTime;
    if (Number.isFinite(started)) for (const animation of animations) animation.startTime = started;
    const record = { animations };
    docks.set(dock, record);
    Promise.all(animations.map(animation => animation.finished)).then(() => {
      if (docks.get(dock) === record) stopDock(dock);
    }).catch(() => {});
  }

  function introduceBrand(element) {
    if (brandIntroduced || !element || !canMove() || !element.animate) return;
    brandIntroduced = true;
    brandAnimation = element.animate([
      { transform: 'perspective(900px) rotateX(3deg) rotateY(-3deg) rotate(-8deg)', opacity: 0.08 },
      { transform: 'perspective(900px) rotateX(0deg) rotateY(0deg) rotate(-8deg)', opacity: 0.15 },
    ], { duration: milliseconds('--motion-player', 280), easing: token('--ease-out', OUT) });
    const animation = brandAnimation;
    animation.finished.finally(() => { if (brandAnimation === animation) brandAnimation = null; }).catch(() => {});
  }

  return { patch, initialize, canMove, reduced, openDialog, closeDialog, setOptions,
    confirmFavorite, captureDock, moveDock, stopDock, dockFrames, introduceBrand };
});
