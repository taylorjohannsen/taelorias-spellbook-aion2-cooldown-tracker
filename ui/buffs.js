const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const overlay = document.getElementById('overlay');
const trackers = new Trackers(overlay.querySelector('.trackers'));
let view = { unlocked: false, editing: false, focused: false, class: 0, combat: false, combatEnded: 0, dungeon: false, buffs: {} };
let settings = null;

function showView(next) {
  // tracked buffs belong to a class, so another character brings its own
  const switched = next.class !== view.class;
  view = next;
  if (switched) invoke('settings').then(applySettings);
  document.body.classList.toggle('unlocked', view.unlocked);
  overlay.toggleAttribute('data-tauri-drag-region', view.unlocked);
  trackers.setBuffs(view.buffs);
}

function applySettings(next) {
  settings = next;
  trackers.setSettings(settings);
  overlay.style.setProperty('--hidden-opacity', settings.hiddenOpacity / 100);
  overlay.style.setProperty('--opacity', settings.opacity / 100);
}

const fit = fitter(overlay, invoke);

function tick() {
  const now = Date.now();
  trackers.tick(now);
  overlay.classList.toggle('empty', trackers.items.size === 0);
  if (settings) {
    overlay.classList.toggle('faded', !visibleNow(view, settings, now));
    document.body.classList.toggle('away', awayNow(view, settings));
  }
  requestAnimationFrame(tick);
}

// listening first, so nothing that changes before the snapshot arrives is missed
Promise.all([listen('view', (event) => showView(event.payload)), listen('settings', (event) => applySettings(event.payload))]).then(() => {
  invoke('snapshot').then(showView);
  invoke('settings').then(applySettings);
});
new ResizeObserver(fit).observe(overlay);
requestAnimationFrame(tick);
