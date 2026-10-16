const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (selector) => document.querySelector(selector);

const stage = $('#stage');
const frameBox = $('#frame-box');
const frame = $('#frame');
const scroller = $('#scroller');
const list = $('#abilities');
const hiddenList = $('#hidden-abilities');
const link = $('#link');
const lock = $('#lock');
const choices = document.querySelectorAll('.segmented[data-key]');
const preview = new Bar(frame);
const buffBox = $('#buff-box');
const buffFrame = $('#buff-frame');
const buffPreview = new Trackers(buffFrame.querySelector('.trackers'));
const trackedList = $('#tracked');
const mineList = $('#mine');
const seenList = $('#seen');

const LOOK = ['orientation', 'iconWidth', 'iconHeight', 'radius', 'gap', 'opacity', 'flashOnUse', 'show', 'hiddenOpacity', 'fadeDelay', 'onlyFocused'];
const PREVIEW_MAX_HEIGHT = 300;
// blank boxes, so sizes can be tried before the character is known
const PLACEHOLDERS = Array.from({ length: 8 }, (_, i) => ({ id: -1 - i, name: ' ', icon: '' }));
// stand-ins for the buff preview until something is tracked
const PLACEHOLDER_BUFFS = [1, 2, 3].map((i) => ({ id: -i, name: `Buff ${i}`, icon: '' }));

let settings = null;
let view = { error: '', character: '', unlocked: false, skills: [], cooldowns: {} };
let skillIds = null;
let seenKey = null;
// every skill of the character's class, passives too, for picking buffs
let classSkills = [];
let activeTab = 'look';
// the buff preview's made-up timers, by buff id
const previewBuffs = {};
// the preview shows the real skills, cooling down on a made-up schedule
const previewView = { error: '', character: '', unlocked: false, skills: PLACEHOLDERS, cooldowns: {} };
// box height / width while the proportion lock is on
let ratio = null;

// This window is the only one that changes settings, so it keeps its own copy
// and sends the whole thing, at most once a frame
let saveQueued = false;
function save() {
  if (saveQueued) return;
  saveQueued = true;
  requestAnimationFrame(() => {
    saveQueued = false;
    invoke('save_settings', { settings });
  });
}

function changed() {
  preview.setSettings(settings);
  buffPreview.setSettings(buffPreviewSettings());
  layoutPreview();
  save();
}

function buffPreviewSettings() {
  return settings.trackedBuffs.length ? settings : { ...settings, trackedBuffs: PLACEHOLDER_BUFFS };
}

const clamp = (value, min, max) => Math.min(max, Math.max(min, value));

// ---------------------------------------------------------------- preview

// The frame sizes itself like the overlay does, and is scaled down if it won't fit.
// The Buffs tab shows the buff trackers instead of the bar.
function layoutPreview() {
  const buffs = activeTab === 'buffs';
  frameBox.hidden = buffs;
  buffBox.hidden = !buffs;
  const [box, el] = buffs ? [buffBox, buffFrame] : [frameBox, frame];
  const w = el.offsetWidth;
  const h = el.offsetHeight;
  const scale = Math.min(1, (stage.clientWidth - 36) / w, PREVIEW_MAX_HEIGHT / h);
  el.style.transform = `scale(${scale})`;
  box.style.width = `${w * scale}px`;
  box.style.height = `${h * scale}px`;
  $('#preview-size').textContent = `${w} × ${h} px${scale < 1 ? ` · shown at ${Math.round(scale * 100)}%` : ''}`;
}

// keep a few boxes cooling so the sweep, timer and key are always on show
function previewCooldowns() {
  const now = Date.now();
  const cooldowns = previewView.cooldowns;
  const shown = preview.shown().map((s) => s.id);
  const ready = shown.filter((id) => !(cooldowns[id]?.endsAt > now));
  if (ready.length && shown.length - ready.length < 3) {
    const id = ready[Math.floor(Math.random() * ready.length)];
    const duration = 4000 + Math.random() * 10000;
    cooldowns[id] = { endsAt: now + duration, duration };
    if (settings?.flashOnUse) preview.cast(id);
  }
  // and buffs that come and go
  if (!settings) return;
  for (const buff of buffPreviewSettings().trackedBuffs) {
    const running = previewBuffs[buff.id];
    if (running && running.endsAt > now) continue;
    if (Math.random() < 0.5) {
      delete previewBuffs[buff.id];
      continue;
    }
    const seen = view.seenBuffs?.find((b) => b.id === buff.id);
    const duration = seen?.duration || 6000 + Math.random() * 12000;
    previewBuffs[buff.id] = { endsAt: now + duration, duration };
  }
}

function tick() {
  const now = Date.now();
  preview.tick(now);
  buffPreview.setBuffs(previewBuffs);
  buffPreview.tick(now);
  requestAnimationFrame(tick);
}

// Npcap missing, or there but not working, which installing it again fixes
const npcapProblem = () => /npcap/i.test(view.error);

// what the waiting screen says, until we know which character is ours
function waiting() {
  if (/npcap (isn't installed|is missing)/i.test(view.error)) {
    return ['Npcap is needed', "tjcd reads the game's traffic through Npcap, the free driver the DPS meters use. Install it, then restart tjcd."];
  }
  if (npcapProblem()) return ["Npcap isn't working", `${view.error}. Installing Npcap again usually fixes it.`];
  if (view.error) return ["Can't read the game", view.error];
  if (!view.traffic) return ['Waiting for AION 2', 'Start the game. tjcd finds it on its own.'];
  return ['Found AION 2', 'Change zone, or use a couple of skills, so tjcd knows which character is yours.'];
}

function showView(next) {
  // tracked buffs belong to a class, so another character brings its own
  if (settings && next.class !== view.class) {
    invoke('settings').then((saved) => {
      settings.trackedBuffs = saved.trackedBuffs;
      buffPreview.setSettings(buffPreviewSettings());
      renderBuffs();
      layoutPreview();
    });
  }
  const opened = next.connected && !view.connected;
  view = next;
  document.body.classList.toggle('connected', Boolean(view.connected));
  const [title, text] = waiting();
  $('#waiting-title').textContent = title;
  $('#waiting-text').textContent = text;
  $('#npcap-help').hidden = !npcapProblem();
  if (opened && settings) requestAnimationFrame(layoutPreview);
  const who = $('#who');
  who.classList.toggle('bad', Boolean(view.error));
  who.classList.toggle('ok', !view.error && view.connected);
  who.title = view.error;
  $('#who-text').textContent = view.error || (view.connected ? view.character || 'Connected' : view.traffic ? 'Finding you' : 'Waiting for game');
  lock.setAttribute('aria-pressed', view.unlocked);
  $('#lock-text').textContent = view.unlocked ? 'Lock in place' : 'Unlock to move';

  const seen = (view.seenBuffs || []).map((b) => `${b.id}:${b.name}:${b.duration}`).join();
  if (seen !== seenKey && settings) {
    seenKey = seen;
    renderBuffs();
  }

  // the view changes with every cast, but only the skills matter here
  const ids = view.skills.map((s) => s.id).join();
  if (ids === skillIds) return;
  skillIds = ids;
  loadClassSkills();
  previewView.skills = view.skills.length ? view.skills : PLACEHOLDERS;
  preview.setView(previewView);
  if (settings) {
    renderAbilities();
    layoutPreview();
  }
}

// ---------------------------------------------------------------- appearance

const fields = [...document.querySelectorAll('.field.slider')].map((el) => {
  const key = el.dataset.key;
  const min = Number(el.dataset.min);
  const max = Number(el.dataset.max);
  const range = el.querySelector('input[type="range"]');
  const num = el.querySelector('input[type="number"]');
  // anything past a slider's end can be typed
  Object.assign(range, { min, max: Number(el.dataset.slider ?? max), step: 1 });
  Object.assign(num, { min, max, step: 1 });
  range.addEventListener('input', () => set(key, Number(range.value)));
  num.addEventListener('input', () => {
    // wait until the number is usable rather than fighting the typing
    if (num.value !== '' && num.validity.valid) set(key, Number(num.value), num);
  });
  num.addEventListener('change', () => sync());
  return { key, range, num };
});

function set(key, value, typing) {
  settings[key] = value;
  if (ratio && key === 'iconWidth') settings.iconHeight = clamp(Math.round(value * ratio), 12, 256);
  if (ratio && key === 'iconHeight') settings.iconWidth = clamp(Math.round(value / ratio), 12, 256);
  sync(typing);
  changed();
}

function sync(typing) {
  for (const { key, range, num } of fields) {
    const value = settings[key];
    if (num !== typing) num.value = value;
    range.value = value;
    const lo = Number(range.min);
    const hi = Number(range.max);
    const share = (clamp(value, lo, hi) - lo) / (hi - lo);
    // the thumb's centre travels 8px in from each end
    range.style.setProperty('--fill', `calc(8px + (100% - 16px) * ${share})`);
  }
  for (const group of choices) {
    for (const button of group.children) button.setAttribute('aria-checked', button.dataset.value === settings[group.dataset.key]);
  }
  for (const field of document.querySelectorAll('.when-hiding')) field.classList.toggle('off', settings.show === 'always');
  const bars = settings.buffStyle === 'bars';
  for (const field of document.querySelectorAll('.when-bars')) field.classList.toggle('off', !bars);
  for (const field of document.querySelectorAll('.when-boxes')) field.classList.toggle('off', bars);
  // a row of boxes grows from the middle; a column from one end
  for (const field of document.querySelectorAll('.when-column')) field.classList.toggle('off', !bars && settings.buffOrientation !== 'vertical');
  link.setAttribute('aria-pressed', Boolean(ratio));
  $('#flash-on-use').checked = settings.flashOnUse;
  $('#only-focused').checked = settings.onlyFocused;
  $('#buff-warning').checked = settings.buffWarning;
  frame.style.opacity = settings.opacity / 100;
  buffFrame.style.opacity = settings.opacity / 100;
}

for (const group of choices) {
  for (const button of group.children) {
    button.addEventListener('click', () => {
      settings[group.dataset.key] = button.dataset.value;
      sync();
      changed();
    });
  }
}

link.addEventListener('click', () => {
  ratio = ratio ? null : settings.iconHeight / settings.iconWidth;
  sync();
});

$('#reset-look').addEventListener('click', async () => {
  const defaults = await invoke('default_settings');
  for (const key of LOOK) settings[key] = defaults[key];
  ratio = settings.iconWidth === settings.iconHeight ? 1 : null;
  sync();
  changed();
});

$('#flash-on-use').addEventListener('change', (e) => {
  settings.flashOnUse = e.target.checked;
  changed();
});

$('#only-focused').addEventListener('change', (e) => {
  settings.onlyFocused = e.target.checked;
  changed();
});

$('#buff-warning').addEventListener('change', (e) => {
  settings.buffWarning = e.target.checked;
  changed();
});

$('#get-npcap').addEventListener('click', () => invoke('open_npcap_download'));
$('#restart').addEventListener('click', () => invoke('restart'));
$('#open-logs').addEventListener('click', () => invoke('open_log_folder'));
invoke('version').then((version) => ($('#version').textContent = `tjcd ${version}`));

// lives in Windows rather than the settings, which says whether it took
const autostart = $('#autostart');
invoke('autostart').then((on) => (autostart.checked = on));
autostart.addEventListener('change', () => invoke('set_autostart', { on: autostart.checked }).then((on) => (autostart.checked = on)));

// the overlay's lock lives in the app; the view says which way it is
lock.addEventListener('click', () => invoke('set_unlocked', { unlocked: !view.unlocked }));

// ---------------------------------------------------------------- tabs

const tabs = document.querySelectorAll('[role="tab"]');
for (const tab of tabs) {
  tab.addEventListener('click', () => {
    for (const other of tabs) {
      const on = other === tab;
      other.setAttribute('aria-selected', on);
      $(`#panel-${other.dataset.tab}`).hidden = !on;
    }
    activeTab = tab.dataset.tab;
    scroller.scrollTop = 0;
    if (settings) layoutPreview();
  });
}

// ---------------------------------------------------------------- abilities

const GRIP = '<svg viewBox="0 0 16 24"><circle cx="5.5" cy="7" r="1.5"/><circle cx="10.5" cy="7" r="1.5"/><circle cx="5.5" cy="12" r="1.5"/><circle cx="10.5" cy="12" r="1.5"/><circle cx="5.5" cy="17" r="1.5"/><circle cx="10.5" cy="17" r="1.5"/></svg>';
const CROSS = '<svg viewBox="0 0 10 10"><path d="M2 2l6 6M8 2l-6 6"/></svg>';
const SPARKLE = '<svg viewBox="0 0 16 16"><path d="M8 1.5l1.6 4.9L14.5 8l-4.9 1.6L8 14.5l-1.6-4.9L1.5 8l4.9-1.6z"/></svg>';

// skill id -> { el, show, key }
const rows = new Map();

function makeRow(skill) {
  const el = document.createElement('li');
  el.className = 'ability';
  el.dataset.id = skill.id;

  const grip = document.createElement('button');
  grip.className = 'grip';
  grip.title = 'Drag to reorder, or use the arrow keys';
  grip.innerHTML = GRIP;
  grip.addEventListener('keydown', (e) => {
    if (e.key !== 'ArrowUp' && e.key !== 'ArrowDown') return;
    e.preventDefault();
    moveBy(skill.id, e.key === 'ArrowUp' ? -1 : 1);
    grip.focus();
  });

  const thumb = document.createElement(skill.icon ? 'img' : 'div');
  thumb.className = 'thumb';
  if (skill.icon) {
    thumb.src = skill.icon;
    thumb.alt = '';
    thumb.draggable = false;
  }

  const name = document.createElement('span');
  name.className = 'name';
  name.textContent = skill.name || `Skill ${skill.id}`;

  const keywrap = document.createElement('div');
  keywrap.className = 'keywrap';
  const key = document.createElement('button');
  key.className = 'hotkey';
  key.addEventListener('click', () => (capturing === skill.id ? stopCapture() : startCapture(skill.id)));
  const clear = document.createElement('button');
  clear.className = 'clear';
  clear.title = 'Clear key';
  clear.innerHTML = CROSS;
  clear.addEventListener('click', () => setHotkey(skill.id, ''));
  keywrap.append(key, clear);

  const pulse = document.createElement('button');
  pulse.className = 'pulse-toggle';
  pulse.title = 'Shine gold when it comes off cooldown';
  pulse.innerHTML = SPARKLE;
  pulse.addEventListener('click', () => {
    const on = !settings.readyPulse.includes(skill.id);
    settings.readyPulse = settings.readyPulse.filter((id) => id !== skill.id);
    if (on) settings.readyPulse.push(skill.id);
    pulse.setAttribute('aria-pressed', on);
    changed();
  });

  const toggle = document.createElement('label');
  toggle.className = 'switch';
  toggle.title = 'Show on the bar';
  const show = document.createElement('input');
  show.type = 'checkbox';
  show.addEventListener('change', () => {
    settings.hidden = settings.hidden.filter((id) => id !== skill.id);
    if (!show.checked) settings.hidden.push(skill.id);
    renderAbilities();
    changed();
  });
  toggle.append(show, document.createElement('span'));

  el.append(grip, thumb, name, keywrap, pulse, toggle);
  return { el, show, key, pulse };
}

function paintKey(row, id) {
  const key = settings.hotkeys[id];
  row.key.textContent = key || 'Set key';
  row.key.classList.toggle('unset', !key);
  row.key.classList.remove('listening');
}

function renderAbilities() {
  const arranged = arrange(view.skills, settings.order);
  const ids = new Set(arranged.map((s) => s.id));
  for (const [id, row] of rows) {
    if (!ids.has(id)) {
      row.el.remove();
      rows.delete(id);
    }
  }
  const hidden = new Set(settings.hidden);
  const shownSkills = arranged.filter((s) => !hidden.has(s.id));
  const hiddenSkills = arranged.filter((s) => hidden.has(s.id));
  // moving a row in the list takes focus off its buttons
  const focused = document.activeElement;
  const place = (skills, into) =>
    skills.forEach((skill, i) => {
      let row = rows.get(skill.id);
      if (!row) {
        row = makeRow(skill);
        rows.set(skill.id, row);
      }
      const on = !hidden.has(skill.id);
      row.show.checked = on;
      row.el.classList.toggle('off', !on);
      row.pulse.setAttribute('aria-pressed', settings.readyPulse.includes(skill.id));
      if (capturing !== skill.id) paintKey(row, skill.id);
      if (into.children[i] !== row.el) into.insertBefore(row.el, into.children[i] || null);
    });
  place(shownSkills, list);
  place(hiddenSkills, hiddenList);
  if (focused && focused.isConnected && document.activeElement !== focused) focused.focus();

  const shown = shownSkills.length;
  const any = arranged.length > 0;
  $('#empty').hidden = any;
  for (const el of document.querySelectorAll('#panel-abilities :is(.toolbar, .tip, .section)')) el.hidden = !any;
  $('#hidden-head').hidden = !hiddenSkills.length;
  $('#none-shown').hidden = !any || shown > 0;
  $('#shown-badge').textContent = shown;
  $('#hidden-badge').textContent = hiddenSkills.length;
  $('#shown-count').textContent = `${shown} of ${arranged.length} shown`;
  $('#count').textContent = any ? shown : '';
  $('#show-all').disabled = shown === arranged.length;
  $('#reset-order').disabled = !arranged.some((s) => settings.order.includes(s.id));
}

// ---------------------------------------------------------------- buffs

const seconds = (ms) => (ms ? `${Math.round(ms / 100) / 10} s` : 'until removed');

function buffRow(buff, tracked) {
  const el = document.createElement('li');
  el.className = 'ability buff';
  const thumb = document.createElement(buff.icon ? 'img' : 'div');
  thumb.className = 'thumb';
  if (buff.icon) {
    thumb.src = buff.icon;
    thumb.alt = '';
    thumb.draggable = false;
    // a few of the game's buff icons aren't on NC's CDN
    thumb.addEventListener('error', () => thumb.replaceWith(Object.assign(document.createElement('div'), { className: 'thumb' })));
  }
  const name = document.createElement('div');
  name.className = 'buff-name';
  const title = document.createElement('span');
  title.className = 'name';
  title.textContent = buff.name || `Buff ${buff.id}`;
  const id = document.createElement('small');
  id.textContent = `#${buff.id}`;
  name.append(title, id);
  const meta = document.createElement('span');
  meta.className = 'buff-meta';
  const seen = view.seenBuffs?.find((b) => b.id === buff.id);
  meta.textContent = seen ? seconds(seen.duration) : '';
  const action = document.createElement('button');
  action.className = 'ghost';
  action.textContent = tracked ? 'Remove' : 'Track';
  action.addEventListener('click', () => {
    if (tracked) settings.trackedBuffs = settings.trackedBuffs.filter((b) => b.id !== buff.id);
    else settings.trackedBuffs.push({ id: buff.id, name: buff.name, icon: buff.icon });
    renderBuffs();
    changed();
  });
  el.append(thumb, name, meta, action);
  return el;
}

function loadClassSkills() {
  invoke('class_skills').then((skills) => {
    classSkills = skills;
    if (settings) renderBuffs();
  });
}

function renderBuffs() {
  const tracked = settings.trackedBuffs;
  const ids = new Set(tracked.map((b) => b.id));
  const own = new Set(classSkills.map((s) => s.id));
  const seenIds = new Set((view.seenBuffs || []).map((b) => b.id));
  // the ones that have buffed us first, otherwise in NC's order
  const mine = classSkills.filter((s) => !ids.has(s.id)).sort((a, b) => seenIds.has(b.id) - seenIds.has(a.id));
  const seen = (view.seenBuffs || []).filter((b) => !ids.has(b.id) && !own.has(b.id));
  trackedList.replaceChildren(...tracked.map((b) => buffRow(b, true)));
  mineList.replaceChildren(...mine.map((b) => buffRow(b, false)));
  seenList.replaceChildren(...seen.map((b) => buffRow(b, false)));
  $('#none-tracked').hidden = tracked.length > 0;
  $('#none-mine').hidden = mine.length > 0 || tracked.some((b) => own.has(b.id));
  $('#none-seen').hidden = seen.length > 0;
  $('#tracked-badge').textContent = tracked.length || '';
  $('#buff-count').textContent = tracked.length || '';
}

function setOrder(ids) {
  // other classes' skills keep their places for when that character logs in
  settings.order = [...ids, ...settings.order.filter((id) => !ids.includes(id))];
  renderAbilities();
  changed();
}

// reordering is for what's on the bar; hidden skills keep their places behind
function moveBy(id, step) {
  const hidden = new Set(settings.hidden);
  const ids = arrange(view.skills, settings.order).map((s) => s.id).filter((i) => !hidden.has(i));
  const from = ids.indexOf(id);
  const to = from + step;
  if (to < 0 || to >= ids.length) return;
  [ids[from], ids[to]] = [ids[to], ids[from]];
  setOrder(ids);
}

$('#show-all').addEventListener('click', () => {
  const ids = new Set(view.skills.map((s) => s.id));
  settings.hidden = settings.hidden.filter((id) => !ids.has(id));
  renderAbilities();
  changed();
});

$('#reset-order').addEventListener('click', () => {
  const ids = new Set(view.skills.map((s) => s.id));
  settings.order = settings.order.filter((id) => !ids.has(id));
  renderAbilities();
  changed();
});

// ---------------------------------------------------------------- drag to reorder

// Pointer events rather than HTML drag and drop, which the webview keeps for file drops
let drag = null;

list.addEventListener('pointerdown', (e) => {
  const row = e.target.closest('.ability');
  if (!row || e.button !== 0 || e.target.closest('.keywrap, .switch')) return;
  drag = {
    row,
    pointerId: e.pointerId,
    startY: e.clientY,
    clientY: e.clientY,
    grab: e.clientY - row.getBoundingClientRect().top,
    active: false,
  };
});

window.addEventListener('pointermove', (e) => {
  if (!drag || e.pointerId !== drag.pointerId) return;
  drag.clientY = e.clientY;
  if (!drag.active) {
    if (Math.abs(e.clientY - drag.startY) < 4) return;
    beginDrag();
  }
  layoutDrag();
});

window.addEventListener('pointerup', endDrag);
window.addEventListener('pointercancel', endDrag);

function beginDrag() {
  stopCapture();
  const all = [...list.children];
  drag.rows = all;
  drag.from = all.indexOf(drag.row);
  drag.to = drag.from;
  drag.stride = all.length > 1 ? all[1].offsetTop - all[0].offsetTop : drag.row.offsetHeight;
  drag.active = true;
  drag.row.classList.add('dragging');
  drag.row.setPointerCapture(drag.pointerId);
  document.body.classList.add('sorting');
  requestAnimationFrame(autoScroll);
}

// the dragged row follows the pointer; the rows it passes slide out of its way
function layoutDrag() {
  const top = drag.clientY - list.getBoundingClientRect().top - drag.grab;
  const y = clamp(top, 0, (drag.rows.length - 1) * drag.stride);
  drag.to = Math.round(y / drag.stride);
  drag.row.style.transform = `translateY(${y - drag.from * drag.stride}px)`;
  drag.rows.forEach((row, i) => {
    if (row === drag.row) return;
    let shift = 0;
    if (i > drag.from && i <= drag.to) shift = -drag.stride;
    else if (i < drag.from && i >= drag.to) shift = drag.stride;
    row.style.transform = shift ? `translateY(${shift}px)` : '';
  });
}

function autoScroll() {
  if (!drag?.active) return;
  const box = scroller.getBoundingClientRect();
  const edge = 48;
  let speed = 0;
  if (drag.clientY < box.top + edge) speed = -Math.ceil((box.top + edge - drag.clientY) / 4);
  else if (drag.clientY > box.bottom - edge) speed = Math.ceil((drag.clientY - box.bottom + edge) / 4);
  if (speed) {
    scroller.scrollTop += speed;
    layoutDrag();
  }
  requestAnimationFrame(autoScroll);
}

function endDrag(e) {
  if (!drag || e.pointerId !== drag.pointerId) return;
  const d = drag;
  drag = null;
  if (!d.active) return;
  document.body.classList.remove('sorting');
  const before = d.row.getBoundingClientRect().top;
  d.row.classList.remove('dragging');
  for (const row of d.rows) {
    row.style.transition = 'none';
    row.style.transform = '';
  }
  const ids = d.rows.map((row) => Number(row.dataset.id));
  const [moved] = ids.splice(d.from, 1);
  ids.splice(d.to, 0, moved);
  setOrder(ids);
  // glide the dropped row from where it was let go into its slot
  d.row.style.transform = `translateY(${before - d.row.getBoundingClientRect().top}px)`;
  void d.row.offsetWidth;
  for (const row of d.rows) row.style.transition = '';
  d.row.style.transform = '';
}

// ---------------------------------------------------------------- hotkeys

// Short names so they fit on a box, with modifiers in front: S1, CQ, AF2, SM4
const KEY_NAMES = {
  Backquote: '`', Minus: '-', Equal: '=', BracketLeft: '[', BracketRight: ']', Backslash: '\\',
  Semicolon: ';', Quote: "'", Comma: ',', Period: '.', Slash: '/', Space: 'Spc', CapsLock: 'Caps',
  Enter: 'Ent', Insert: 'Ins', Delete: 'Del', Home: 'Home', End: 'End', PageUp: 'PgU', PageDown: 'PgD',
  ArrowUp: 'Up', ArrowDown: 'Dn', ArrowLeft: 'Lt', ArrowRight: 'Rt',
  NumpadAdd: 'N+', NumpadSubtract: 'N-', NumpadMultiply: 'N*', NumpadDivide: 'N/', NumpadDecimal: 'N.', NumpadEnter: 'NEnt',
};
const MODIFIER_KEYS = ['Shift', 'Control', 'Alt', 'Meta'];

let capturing = null;

function modifiers(e) {
  return (e.ctrlKey ? 'C' : '') + (e.altKey ? 'A' : '') + (e.shiftKey ? 'S' : '');
}

function keyName(code) {
  return KEY_NAMES[code] ?? code.replace(/^(Key|Digit)/, '').replace(/^Numpad/, 'N');
}

function startCapture(id) {
  stopCapture();
  capturing = id;
  const { key } = rows.get(id);
  key.classList.add('listening');
  key.textContent = 'Press a key';
}

function stopCapture() {
  if (capturing === null) return;
  const id = capturing;
  capturing = null;
  const row = rows.get(id);
  if (row) paintKey(row, id);
}

function setHotkey(id, key) {
  if (key) settings.hotkeys[id] = key;
  else delete settings.hotkeys[id];
  if (capturing === id) capturing = null;
  const row = rows.get(id);
  if (row) paintKey(row, id);
  changed();
}

document.addEventListener(
  'keydown',
  (e) => {
    if (capturing === null) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.key === 'Escape') return stopCapture();
    if (MODIFIER_KEYS.includes(e.key)) {
      const held = [e.ctrlKey && 'Ctrl', e.altKey && 'Alt', e.shiftKey && 'Shift'].filter(Boolean);
      rows.get(capturing).key.textContent = `${held.join(' + ')} + …`;
      return;
    }
    const mods = modifiers(e);
    const clear = !mods && (e.key === 'Backspace' || e.key === 'Delete');
    setHotkey(capturing, clear ? '' : mods + keyName(e.code));
  },
  true,
);

// middle, back and forward buttons on the listening key; a click anywhere else cancels
document.addEventListener(
  'mousedown',
  (e) => {
    if (capturing === null) return;
    const { key } = rows.get(capturing);
    if (e.target === key && (e.button === 1 || e.button >= 3)) {
      e.preventDefault();
      setHotkey(capturing, `${modifiers(e)}M${e.button === 1 ? 3 : e.button + 1}`);
    } else if (e.target !== key) {
      stopCapture();
    }
  },
  true,
);

window.addEventListener('blur', stopCapture);

// ---------------------------------------------------------------- start

listen('view', (event) => showView(event.payload))
  .then(() => Promise.all([invoke('settings'), invoke('snapshot')]))
  .then(([saved, snapshot]) => {
    settings = saved;
    ratio = settings.iconWidth === settings.iconHeight ? 1 : null;
    sync();
    preview.setSettings(settings);
    buffPreview.setSettings(buffPreviewSettings());
    showView(snapshot);
    renderAbilities();
    renderBuffs();
    layoutPreview();
    // next frame, since laying out the preview changes the stage it watches
    new ResizeObserver(() => requestAnimationFrame(layoutPreview)).observe(stage);
  });
preview.setView(previewView);
previewCooldowns();
previewCooldowns();
setInterval(previewCooldowns, 1200);
requestAnimationFrame(tick);
