// Wspólne dla wszystkich okien: połączenie z backendem, migawka stanu,
// formatowanie i ikony.

const T = window.__TAURI__;

export const invoke = (cmd, args = {}) => T.core.invoke(cmd, args);
export const listen = (event, cb) => T.event.listen(event, (e) => cb(e.payload));
export const windowLabel = T.webviewWindow.getCurrentWebviewWindow().label;

let snap = null;
const subscribers = [];
const tickers = [];

function apply(s) {
  snap = s;
  document.documentElement.style.setProperty('--accent', s.mode.accent);
  for (const cb of subscribers) cb(s);
}

/** Wywołuje `cb` przy każdej nowej migawce stanu (i od razu, jeśli już jest). */
export function onSnapshot(cb) {
  subscribers.push(cb);
  if (snap) cb(snap);
}

/** Wywołuje `cb` co sekundę z ostatnią migawką — do liczników czasu. */
export function onTick(cb) {
  tickers.push(cb);
}

export const current = () => snap;

listen('snapshot', apply);
invoke('get_snapshot').then(apply);
setInterval(() => { if (snap) for (const cb of tickers) cb(snap); }, 1000);

// Esc chowa okno (poza paskiem i dockiem).
if (!['topbar', 'dock', 'glow', 'settings'].includes(windowLabel)) {
  window.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') invoke('hide_window', { label: windowLabel });
  });
}

export function esc(value) {
  return String(value ?? '').replace(/[&<>"']/g, (c) => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
  })[c]);
}

export function hhmm(iso) {
  if (!iso) return '';
  const d = new Date(iso);
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}

export function mmss(ms) {
  const total = Math.max(0, Math.round(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  return h > 0
    ? `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`
    : `${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
}

export function minutesUntil(iso) {
  return Math.round((new Date(iso) - Date.now()) / 60000);
}

/** Stan sesji fokusu: ile zostało, ile trwa, jaki postęp. */
export function focusTiming(s) {
  const f = s.focus;
  if (!f || !f.start || !f.end) return null;
  const start = new Date(f.start).getTime();
  const end = new Date(f.end).getTime();
  const now = s.paused && s.pausedSince ? new Date(s.pausedSince).getTime() : Date.now();
  const total = Math.max(1, end - start);
  const left = Math.max(0, end - now);
  return { left, total, progress: Math.min(1, Math.max(0, 1 - left / total)) };
}

export const KIND_LABEL = {
  task: 'zadanie', habit: 'nawyk', focus: 'fokus', buffer: 'bufor', meeting: 'spotkanie', busy: 'zajęte',
};

/** Kolor klocka dnia: fioletowy przed czasem, zielony w czasie, żółty po czasie. */
export function blockColor(item, accent) {
  switch (item.status) {
    case 'early': return 'var(--purple)';
    case 'ontime': return 'var(--green)';
    case 'late': return 'var(--yellow)';
    case 'past': return '#3f3f46';
    case 'now': return accent;
    default:
      if (item.kind === 'meeting' || item.kind === 'busy') return 'rgba(251,191,36,0.75)';
      if (item.kind === 'buffer') return '#52525b';
      if (item.kind === 'habit') return 'rgba(74,222,128,0.55)';
      return 'rgba(255,255,255,0.28)';
  }
}

export function toast(text, ms = 1800) {
  let el = document.querySelector('.toast');
  if (!el) {
    el = document.createElement('div');
    el.className = 'toast';
    document.body.appendChild(el);
  }
  el.textContent = text;
  el.classList.add('show');
  clearTimeout(el._t);
  el._t = setTimeout(() => el.classList.remove('show'), ms);
}

const P = (d) => `<path d="${d}"></path>`;
const ICONS = {
  folder: P('M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z'),
  globe: '<circle cx="12" cy="12" r="9"></circle>' + P('M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18'),
  sheet: '<rect x="3" y="4" width="18" height="16" rx="2"></rect>' + P('M3 10h18M9 4v16'),
  terminal: P('M4 17l6-5-6-5M12 19h8'),
  mail: '<rect x="3" y="5" width="18" height="14" rx="2"></rect>' + P('M3 7l9 6 9-6'),
  clock: '<circle cx="12" cy="13" r="8"></circle>' + P('M12 9v4l2 2M9 2h6'),
  chat: P('M4 5h16v11H9l-5 4z'),
  note: P('M5 4h14v11l-5 5H5z') + P('M14 20v-5h5'),
  music: P('M9 18V5l12-2v13') + '<circle cx="6" cy="18" r="3"></circle><circle cx="18" cy="16" r="3"></circle>',
  video: '<rect x="3" y="5" width="18" height="14" rx="3"></rect>' + P('M10 9l5 3-5 3z'),
  game: '<rect x="2" y="7" width="20" height="11" rx="5"></rect>' + P('M7 11v3M5.5 12.5h3M16 12h.01M18 14h.01'),
  code: P('M8 7l-5 5 5 5M16 7l5 5-5 5'),
  image: '<rect x="3" y="4" width="18" height="16" rx="2"></rect><circle cx="9" cy="10" r="2"></circle>' + P('M21 16l-5-5-9 9'),
  calc: '<rect x="5" y="3" width="14" height="18" rx="2"></rect>' + P('M8 7h8M8 12h.01M12 12h.01M16 12h.01M8 16h.01M12 16h.01M16 16h.01'),
  settings: '<circle cx="12" cy="12" r="3"></circle>' + P('M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z'),
  search: '<circle cx="11" cy="11" r="7"></circle>' + P('M20 20l-4-4'),
  apps: '<rect x="4" y="4" width="6" height="6" rx="1.5"></rect><rect x="14" y="4" width="6" height="6" rx="1.5"></rect><rect x="4" y="14" width="6" height="6" rx="1.5"></rect><rect x="14" y="14" width="6" height="6" rx="1.5"></rect>',
  clipboard: '<rect x="8" y="3" width="8" height="4" rx="1"></rect>' + P('M8 5H6a1 1 0 0 0-1 1v14a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V6a1 1 0 0 0-1-1h-2'),
  stack: '<rect x="3" y="8" width="14" height="12" rx="2"></rect>' + P('M7 8V6a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2h-2'),
  left: P('M15 6l-6 6 6 6'),
  right: P('M9 6l6 6-6 6'),
  close: P('M6 6l12 12M18 6L6 18'),
  check: P('M5 12l5 5 9-10'),
  calendar: '<rect x="3" y="5" width="18" height="16" rx="2"></rect>' + P('M3 10h18M8 3v4M16 3v4'),
  bell: P('M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9') + P('M10 21h4'),
  wifi: P('M2 9a15 15 0 0 1 20 0M5 12.5a10 10 0 0 1 14 0M8.5 16a5 5 0 0 1 7 0') + P('M12 19.5h.01'),
  battery: '<rect x="2" y="7" width="18" height="10" rx="2"></rect>' + P('M22 11v2'),
  download: P('M12 4v11M7 10l5 5 5-5M5 20h14'),
  file: P('M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z') + P('M14 3v5h5'),
  camera: P('M4 8V5h3M17 5h3v3M20 16v3h-3M7 19H4v-3') + '<circle cx="12" cy="12" r="3"></circle>',
  replan: P('M3 12a9 9 0 0 1 15.5-6.3L21 8') + P('M21 3v5h-5') + P('M21 12a9 9 0 0 1-15.5 6.3L3 16') + P('M3 21v-5h5'),
  cpu: '<rect x="6" y="6" width="12" height="12" rx="2"></rect><rect x="9.5" y="9.5" width="5" height="5" rx="1"></rect>' + P('M9 2v4M15 2v4M9 18v4M15 18v4M2 9h4M2 15h4M18 9h4M18 15h4'),
  ram: '<rect x="3" y="7" width="18" height="10" rx="1.5"></rect>' + P('M7 17v3M11 17v3M15 17v3M7 10v4M11 10v4M15 10v4'),
  volume: P('M4 9v6h4l5 4V5L8 9z') + P('M16 9a4 4 0 0 1 0 6M18.5 6.5a8 8 0 0 1 0 11'),
  mute: P('M4 9v6h4l5 4V5L8 9z') + P('M17 9l5 6M22 9l-5 6'),
  bluetooth: P('M7 7l10 10-5 4V3l5 4L7 17'),
  down: P('M12 5v14M6 13l6 6 6-6'),
  up: P('M12 19V5M6 11l6-6 6 6'),
  plus: P('M12 5v14M5 12h14'),
  trash: P('M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3'),
  drag: '<circle cx="9" cy="6" r="1"></circle><circle cx="15" cy="6" r="1"></circle><circle cx="9" cy="12" r="1"></circle><circle cx="15" cy="12" r="1"></circle><circle cx="9" cy="18" r="1"></circle><circle cx="15" cy="18" r="1"></circle>',
  pin: P('M12 17v5M5 17h14l-2-5V5H7v7z'),
  keyboard: '<rect x="2" y="6" width="20" height="12" rx="2"></rect>' + P('M6 10h.01M10 10h.01M14 10h.01M18 10h.01M7 14h10'),
  radio: '<circle cx="12" cy="12" r="2"></circle>' + P('M16.2 7.8a6 6 0 0 1 0 8.4M7.8 16.2a6 6 0 0 1 0-8.4M19 5a10 10 0 0 1 0 14M5 19A10 10 0 0 1 5 5'),
  layers: P('M12 3l9 5-9 5-9-5z') + P('M3 13l9 5 9-5'),
  power: P('M12 3v9') + P('M6.4 6.4a8 8 0 1 0 11.2 0'),
  lock: '<rect x="5" y="11" width="14" height="10" rx="2"></rect>' + P('M8 11V8a4 4 0 0 1 8 0v3'),
  moon: P('M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z'),
  restart: P('M3 12a9 9 0 1 0 3-6.7L3 8') + P('M3 3v5h5'),
  logout: P('M15 4h3a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2h-3') + P('M10 17l-5-5 5-5M5 12h11'),
  snow: P('M12 2v20M4.9 7l14.2 10M4.9 17L19.1 7'),
  heart: P('M12 20s-7-4.4-9.3-9A5 5 0 0 1 12 6a5 5 0 0 1 9.3 5c-2.3 4.6-9.3 9-9.3 9z'),
  shuffle: P('M16 3h5v5M4 20L21 3M21 16v5h-5M15 15l6 6M4 4l5 5'),
  repeat: P('M17 2l4 4-4 4') + P('M3 11V9a3 3 0 0 1 3-3h15') + P('M7 22l-4-4 4-4') + P('M21 13v2a3 3 0 0 1-3 3H3'),
  wifi_lock: P('M12 15v2'),
  send: P('M22 2L11 13M22 2l-7 20-4-9-9-4z'),
  sliders: P('M4 6h10M18 6h2M4 12h4M12 12h8M4 18h12M20 18h0') + '<circle cx="16" cy="6" r="2"></circle><circle cx="10" cy="12" r="2"></circle><circle cx="18" cy="18" r="2"></circle>',
};

export const BUILTIN_ICONS = ['folder', 'globe', 'sheet', 'terminal', 'mail', 'clock', 'chat', 'note', 'music', 'video', 'game', 'code', 'image', 'calc', 'settings'];
const FILLED = {
  play: '<path d="M8 5l11 7-11 7z"></path>',
  pause: '<rect x="6" y="5" width="4" height="14"></rect><rect x="14" y="5" width="4" height="14"></rect>',
  next: '<path d="M16 5h2v14h-2zM4 5v14l11-7z"></path>',
  prev: '<path d="M6 5h2v14H6zM20 5v14L9 12z"></path>',
};

/** Ikona SVG jako HTML. Nieznana nazwa → monogram (1–2 litery). */
export function icon(name, size = 18, stroke = 1.8) {
  if (FILLED[name]) {
    return `<svg aria-hidden="true" width="${size}" height="${size}" viewBox="0 0 24 24" fill="currentColor">${FILLED[name]}</svg>`;
  }
  if (ICONS[name]) {
    return `<svg aria-hidden="true" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="${stroke}" stroke-linecap="round" stroke-linejoin="round">${ICONS[name]}</svg>`;
  }
  return `<span aria-hidden="true" style="font-weight:700;font-size:${Math.round(size * 0.8)}px">${esc(String(name || '?').slice(0, 2).toUpperCase())}</span>`;
}

/** Pierścień postępu (SVG) — jak w panelu Teraz i dymku. */
export function ring(size, stroke, progress, color, inner = '') {
  const r = (size - stroke) / 2;
  const c = 2 * Math.PI * r;
  const filled = c * (1 - Math.min(1, Math.max(0, progress)));
  return `<svg width="${size}" height="${size}" viewBox="0 0 ${size} ${size}" role="img" aria-hidden="true">
    <circle cx="${size / 2}" cy="${size / 2}" r="${r}" fill="none" stroke="rgba(255,255,255,0.08)" stroke-width="${stroke}"></circle>
    <circle cx="${size / 2}" cy="${size / 2}" r="${r}" fill="none" stroke="${color}" stroke-width="${stroke}" stroke-linecap="round"
      stroke-dasharray="${filled.toFixed(1)} ${c.toFixed(1)}" transform="rotate(-90 ${size / 2} ${size / 2})"></circle>
    ${inner}
  </svg>`;
}
