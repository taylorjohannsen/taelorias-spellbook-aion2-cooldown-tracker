// The skill bar, drawn the same way by the overlay and the settings preview

// Skills in the order they were arranged in settings, the rest after them in bar order
function arrange(skills, order) {
  const rank = new Map(order.map((id, i) => [id, i]));
  return [...skills].sort((a, b) => (rank.get(a.id) ?? order.length) - (rank.get(b.id) ?? order.length));
}

// Whether the overlays should be up, by the Show setting. Unlocked or being set up, they always are.
function visibleNow(view, settings, now) {
  if (view.unlocked || view.editing) return true;
  const fighting = view.combat || now - view.combatEnded < settings.fadeDelay * 1000;
  switch (settings.show) {
    case 'combat':
      return fighting;
    case 'dungeon':
      return view.dungeon;
    case 'dungeonCombat':
      return view.dungeon && fighting;
    default:
      return true;
  }
}

// Out of sight while another program is in front, unless it's being moved
function awayNow(view, settings) {
  return Boolean(settings.onlyFocused) && !view.focused && !view.unlocked;
}

// An overlay window is always the size of what it shows
function fitter(el, invoke) {
  let fitted = '';
  return () => {
    const width = Math.ceil(el.offsetWidth);
    const height = Math.ceil(el.offsetHeight);
    if (`${width}x${height}` === fitted) return;
    fitted = `${width}x${height}`;
    invoke('fit_window', { width, height });
  };
}

function formatTime(ms) {
  const s = ms / 1000;
  if (s >= 60) return `${Math.ceil(s / 60)}m`;
  if (s >= 3) return String(Math.ceil(s));
  return s.toFixed(1);
}

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
  const ring = document.createElement('div');
  ring.className = 'ring';
  const pulse = document.createElement('div');
  pulse.className = 'pulse';
  const time = document.createElement('div');
  time.className = 'time';
  const key = document.createElement('div');
  key.className = 'key';
  el.append(sweep, ring, pulse, time, key);
  return { el, sweep, time, key, cooling: false };
}

class Bar {
  // `frame` holds a .bar and a .status
  constructor(frame) {
    this.frame = frame;
    this.bar = frame.querySelector('.bar');
    this.status = frame.querySelector('.status');
    // skill id -> { el, sweep, time, key, cooling }
    this.icons = new Map();
    this.view = { error: '', character: '', unlocked: false, skills: [], cooldowns: {} };
    this.settings = { order: [], hidden: [], hotkeys: {} };
    // skills that pulse gold as they come back
    this.pulsing = new Set();
  }

  setView(view) {
    this.view = view;
    this.render();
  }

  setSettings(settings) {
    this.settings = settings;
    const style = this.frame.style;
    style.setProperty('--icon-w', `${settings.iconWidth}px`);
    style.setProperty('--icon-h', `${settings.iconHeight}px`);
    style.setProperty('--radius', `${settings.radius}px`);
    style.setProperty('--gap', `${settings.gap}px`);
    this.frame.classList.toggle('vertical', settings.orientation === 'vertical');
    this.pulsing = new Set(settings.readyPulse || []);
    this.render();
  }

  shown() {
    const hidden = new Set(this.settings.hidden);
    return arrange(this.view.skills, this.settings.order).filter((s) => !hidden.has(s.id));
  }

  render() {
    const shown = this.shown();
    const ids = new Set(shown.map((s) => s.id));
    for (const [id, icon] of this.icons) {
      if (!ids.has(id)) {
        icon.el.remove();
        this.icons.delete(id);
      }
    }
    shown.forEach((skill, i) => {
      let icon = this.icons.get(skill.id);
      if (!icon) {
        icon = makeIcon(skill);
        this.icons.set(skill.id, icon);
      }
      icon.key.textContent = this.settings.hotkeys[skill.id] || '';
      if (this.bar.children[i] !== icon.el) this.bar.insertBefore(icon.el, this.bar.children[i] || null);
    });

    const view = this.view;
    let text = view.error;
    if (!text && !view.skills.length) text = view.character || 'Waiting for game';
    if (!text && view.unlocked) text = view.character;
    this.status.textContent = text;
    this.status.classList.toggle('error', Boolean(view.error));
  }

  // we used a skill
  cast(id) {
    const icon = this.icons.get(id);
    if (!icon) return;
    icon.el.classList.remove('cast');
    // restart the animation for a quick second use
    void icon.el.offsetWidth;
    icon.el.classList.add('cast');
  }

  tick(now) {
    for (const [id, icon] of this.icons) {
      const cd = this.view.cooldowns[id];
      const left = cd ? cd.endsAt - now : 0;
      if (left > 0) {
        if (!icon.cooling) {
          icon.cooling = true;
          icon.el.classList.remove('ready');
          icon.el.classList.add('cooling');
        }
        const elapsed = Math.max(0, 1 - left / cd.duration);
        icon.sweep.style.setProperty('--elapsed', `${elapsed * 360}deg`);
        icon.time.textContent = formatTime(left);
      } else if (icon.cooling) {
        icon.cooling = false;
        icon.el.classList.remove('cooling');
        // restart the ready flash, and the pulse for the ones that want it
        icon.el.classList.remove('pulsing');
        void icon.el.offsetWidth;
        icon.el.classList.add('ready');
        if (this.pulsing.has(id)) icon.el.classList.add('pulsing');
      }
    }
  }
}
