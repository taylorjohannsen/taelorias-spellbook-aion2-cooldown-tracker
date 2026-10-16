// Buff trackers, drawn the same way by the buffs window and the settings preview.
// Boxes count down like the skill bar; bars drain from full.

// how long before a buff runs out it's outlined in red
const ENDING_MS = 3000;

function makeTracker(buff) {
  const name = buff.name || `Buff ${buff.id}`;
  const el = document.createElement('div');
  el.className = 'tracker';
  el.title = name;

  const icon = document.createElement('div');
  icon.className = 'tracker-icon';
  const initials = document.createElement('span');
  initials.className = 'tracker-label';
  initials.textContent = buff.name ? buff.name.slice(0, 2) : '?';
  if (buff.icon) {
    const img = document.createElement('img');
    img.src = buff.icon;
    img.alt = '';
    img.draggable = false;
    // a few of the game's buff icons aren't on NC's CDN
    img.addEventListener('error', () => img.replaceWith(initials));
    icon.append(img);
  } else {
    icon.append(initials);
  }
  const sweep = document.createElement('div');
  sweep.className = 'tracker-sweep';
  const time = document.createElement('div');
  time.className = 'tracker-time';
  icon.append(sweep, time);

  const bar = document.createElement('div');
  bar.className = 'tracker-bar';
  const label = document.createElement('span');
  label.className = 'tracker-name';
  label.textContent = name;
  const left = document.createElement('span');
  left.className = 'tracker-left';
  bar.append(label, left);

  el.append(icon, bar);
  return { el, time, left };
}

class Trackers {
  // `root` lays the trackers out and sizes itself to them
  constructor(root) {
    this.root = root;
    // buff id -> { el, time, left }
    this.items = new Map();
    // buff id -> { endsAt, duration }; endsAt 0 lasts until it's removed
    this.buffs = {};
    this.settings = { trackedBuffs: [] };
    // how many are on show after the last tick
    this.shown = 0;
  }

  setBuffs(buffs) {
    this.buffs = buffs;
  }

  setSettings(settings) {
    this.settings = settings;
    const style = this.root.style;
    style.setProperty('--buff-size', `${settings.buffSize}px`);
    style.setProperty('--buff-bar-width', `${settings.buffBarWidth}px`);
    style.setProperty('--radius', `${settings.radius}px`);
    style.setProperty('--gap', `${settings.gap}px`);
    const bars = settings.buffStyle === 'bars';
    this.root.classList.toggle('bars', bars);
    this.root.classList.toggle('column', bars || settings.buffOrientation === 'vertical');
    this.root.classList.toggle('up', settings.buffGrow === 'up');
    this.render();
  }

  render() {
    const tracked = this.settings.trackedBuffs;
    // sizes the row or column for all of them, shown or not
    this.root.style.setProperty('--count', tracked.length);
    const ids = new Set(tracked.map((b) => b.id));
    for (const [id, item] of this.items) {
      if (!ids.has(id)) {
        item.el.remove();
        this.items.delete(id);
      }
    }
    tracked.forEach((buff, i) => {
      let item = this.items.get(buff.id);
      if (!item) {
        item = makeTracker(buff);
        this.items.set(buff.id, item);
      }
      if (this.root.children[i] !== item.el) this.root.insertBefore(item.el, this.root.children[i] || null);
    });
  }

  tick(now) {
    let shown = 0;
    for (const [id, item] of this.items) {
      const buff = this.buffs[id];
      const on = Boolean(buff) && (buff.endsAt === 0 || buff.endsAt > now);
      const left = on && buff.endsAt ? buff.endsAt - now : 0;
      // the part that's left, full for one that lasts until it's removed
      const share = !on ? 0 : buff.endsAt ? Math.min(1, left / buff.duration) : 1;
      const gone = !on && this.settings.buffMissing === 'hide';
      item.el.classList.toggle('ending', this.settings.buffWarning !== false && left > 0 && left <= ENDING_MS);
      item.el.classList.toggle('missing', !on);
      item.el.classList.toggle('gone', gone);
      if (!gone) shown += 1;
      item.el.style.setProperty('--left', share);
      const text = left ? formatTime(left) : '';
      if (item.time.textContent !== text) {
        item.time.textContent = text;
        item.left.textContent = text;
      }
    }
    this.shown = shown;
  }
}
