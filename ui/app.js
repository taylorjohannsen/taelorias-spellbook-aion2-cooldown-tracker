const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const overlay = document.getElementById('overlay');
const bar = document.getElementById('bar');
const status = document.getElementById('status');

// skill id -> { el, sweep, time, cooling }
const icons = new Map();
let view = { error: '', character: '', unlocked: false, skills: [], cooldowns: {} };

function makeIcon(skill) {
  const el = document.createElement('div');
  el.className = 'icon';
  if (skill.icon) {
    const img = document.createElement('img');
    img.src = skill.icon;
    img.alt = skill.name;
    img.draggable = false;
    el.append(img);
  } else {
    const label = document.createElement('span');
    label.className = 'label';
    label.textContent = skill.name || String(skill.id).slice(0, 4);
    el.append(label);
  }
  const sweep = document.createElement('div');
  sweep.className = 'sweep';
  const time = document.createElement('div');
  time.className = 'time';
  el.append(sweep, time);
  return { el, sweep, time, cooling: false };
}

function render(next) {
  view = next;
  document.body.classList.toggle('unlocked', view.unlocked);
  overlay.toggleAttribute('data-tauri-drag-region', view.unlocked);

  const ids = new Set(view.skills.map((s) => s.id));
  for (const [id, icon] of icons) {
    if (!ids.has(id)) {
      icon.el.remove();
      icons.delete(id);
    }
  }
  view.skills.forEach((skill, i) => {
    let icon = icons.get(skill.id);
    if (!icon) {
      icon = makeIcon(skill);
      icons.set(skill.id, icon);
    }
    if (bar.children[i] !== icon.el) bar.insertBefore(icon.el, bar.children[i] || null);
  });

  let text = view.error;
  if (!text && !view.skills.length) text = view.character || 'Waiting for game';
  if (!text && view.unlocked) text = view.character;
  status.textContent = text;
  status.classList.toggle('error', Boolean(view.error));
}

function format(ms) {
  const s = ms / 1000;
  if (s >= 60) return `${Math.ceil(s / 60)}m`;
  if (s >= 3) return String(Math.ceil(s));
  return s.toFixed(1);
}

function tick() {
  const now = Date.now();
  for (const [id, icon] of icons) {
    const cd = view.cooldowns[id];
    const left = cd ? cd.endsAt - now : 0;
    if (left > 0) {
      if (!icon.cooling) {
        icon.cooling = true;
        icon.el.classList.remove('ready');
        icon.el.classList.add('cooling');
      }
      const elapsed = Math.max(0, 1 - left / cd.duration);
      icon.sweep.style.setProperty('--elapsed', `${elapsed * 360}deg`);
      icon.time.textContent = format(left);
    } else if (icon.cooling) {
      icon.cooling = false;
      icon.el.classList.remove('cooling');
      // restart the ready flash
      void icon.el.offsetWidth;
      icon.el.classList.add('ready');
    }
  }
  requestAnimationFrame(tick);
}

listen('view', (event) => render(event.payload));
invoke('snapshot').then(render);
requestAnimationFrame(tick);
