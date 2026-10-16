const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const overlay = document.getElementById('overlay');
const bar = new Bar(overlay);

function showView(view) {
  document.body.classList.toggle('unlocked', view.unlocked);
  overlay.toggleAttribute('data-tauri-drag-region', view.unlocked);
  bar.setView(view);
}

function applySettings(settings) {
  bar.setSettings(settings);
  overlay.style.setProperty('--hidden-opacity', settings.hiddenOpacity / 100);
  overlay.style.setProperty('--opacity', settings.opacity / 100);
}

const fit = fitter(overlay, invoke);

function tick() {
  const now = Date.now();
  bar.tick(now);
  overlay.classList.toggle('faded', !visibleNow(bar.view, bar.settings, now));
  document.body.classList.toggle('away', awayNow(bar.view, bar.settings));
  requestAnimationFrame(tick);
}

// listening first, so nothing that changes before the snapshot arrives is missed
Promise.all([
  listen('view', (event) => showView(event.payload)),
  listen('settings', (event) => applySettings(event.payload)),
  listen('cast', (event) => {
    if (bar.settings.flashOnUse) bar.cast(event.payload);
  }),
]).then(() => {
  invoke('snapshot').then(showView);
  invoke('settings').then(applySettings);
});
new ResizeObserver(fit).observe(overlay);
requestAnimationFrame(tick);
