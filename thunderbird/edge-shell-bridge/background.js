// Edge Shell Bridge — most między Thunderbirdem a Edge Shell (127.0.0.1).
//
// Thunderbird → Edge Shell:
//   POST /mail/event   nowy mail w skrzynce odbiorczej
//   POST /mail/state   nieprzeczytane na konto + oflagowane
//   POST /mail/result  odpowiedź na polecenie z reqId (szukaj, kontakty, skrzynka)
// Edge Shell → Thunderbird:
//   GET  /mail/commands  co ~0,8 s; zwraca listę poleceń (to też „jestem”)

const DEF = { port: 7311, token: '' };
let cfg = { ...DEF };

async function loadCfg() {
  try { cfg = { ...DEF, ...(await browser.storage.local.get(DEF)) }; } catch { cfg = { ...DEF }; }
}
browser.storage.onChanged.addListener(loadCfg);

const url = (p) => `http://127.0.0.1:${cfg.port}${p}`;
function headers() {
  const h = { 'Content-Type': 'application/json' };
  if (cfg.token) h.Authorization = `Bearer ${cfg.token}`;
  return h;
}
async function post(path, body) {
  try {
    await fetch(url(path), { method: 'POST', headers: headers(), body: JSON.stringify(body) });
  } catch { /* Edge Shell nie działa — spróbujemy później */ }
}

// ------------------------------------------------------------------ pomocnicze

const isInbox = (f) => !!f && (f.type === 'inbox' || (f.specialUse || []).includes('inbox'));

function findInbox(folders) {
  for (const f of folders || []) {
    if (isInbox(f)) return f;
    const sub = findInbox(f.subFolders);
    if (sub) return sub;
  }
  return null;
}

async function* iterate(list) {
  let page = list;
  while (page) {
    for (const m of page.messages || []) yield m;
    page = page.id ? await browser.messages.continueList(page.id) : null;
  }
}

function htmlToText(html) {
  return String(html || '')
    .replace(/<(style|script)[^>]*>[\s\S]*?<\/\1>/gi, ' ')
    .replace(/<br\s*\/?>/gi, '\n')
    .replace(/<\/(p|div|tr|li|h\d)>/gi, '\n')
    .replace(/<[^>]+>/g, ' ')
    .replace(/&nbsp;/g, ' ')
    .replace(/&amp;/g, '&')
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'");
}

// Pierwszy tekst z wiadomości: text/plain, a gdy go nie ma — oczyszczony HTML.
function bodyText(part) {
  let plain = '';
  let html = '';
  const walk = (p) => {
    if (!p) return;
    const ct = (p.contentType || '').toLowerCase();
    if (!plain && ct === 'text/plain' && p.body) plain = p.body;
    if (!html && ct === 'text/html' && p.body) html = p.body;
    for (const c of p.parts || []) walk(c);
  };
  walk(part);
  const text = plain || htmlToText(html);
  // Bez cytatów („>”) i podpisu, jedna linijka.
  return text
    .split('\n')
    .filter((l) => !l.trim().startsWith('>'))
    .join('\n')
    .split(/\n-- \n/)[0]
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, 400);
}

const accountNames = new Map();
async function accountName(id) {
  if (!accountNames.has(id)) {
    try { accountNames.set(id, (await browser.accounts.get(id)).name); } catch { accountNames.set(id, ''); }
  }
  return accountNames.get(id);
}

function header(h, name) {
  const v = h?.[name];
  return Array.isArray(v) ? v[0] || '' : v || '';
}

async function describe(m, withBody) {
  const out = {
    id: m.id,
    mid: m.headerMessageId || '',
    account: m.folder?.accountId || '',
    accountName: await accountName(m.folder?.accountId),
    author: m.author || '',
    subject: m.subject || '',
    date: m.date ? new Date(m.date).toISOString() : '',
    flagged: !!m.flagged,
    read: !!m.read,
    snippet: '',
    newsletter: false,
  };
  if (withBody) {
    try {
      const full = await browser.messages.getFull(m.id);
      const h = full.headers || {};
      out.snippet = bodyText(full);
      out.newsletter = !!(header(h, 'list-unsubscribe') || header(h, 'list-id') || /bulk|list/i.test(header(h, 'precedence')));
    } catch { /* bez treści */ }
  }
  return out;
}

// ------------------------------------------------------------------ nowa poczta

browser.messages.onNewMailReceived.addListener(async (folder, list) => {
  if (!isInbox(folder)) return;
  for await (const m of iterate(list)) {
    if (m.junk) continue;
    await post('/mail/event', { kind: 'new', mail: await describe(m, true) });
  }
  scheduleState();
});

// ------------------------------------------------------------------ liczniki i oflagowane

let stateTimer = null;
function scheduleState() {
  clearTimeout(stateTimer);
  stateTimer = setTimeout(sendState, 1500);
}

async function inboxes() {
  const out = [];
  let accounts = [];
  try { accounts = await browser.accounts.list(true); } catch { accounts = await browser.accounts.list(); }
  for (const a of accounts) {
    const inbox = findInbox(a.folders);
    if (inbox) out.push({ account: a, inbox });
  }
  return out;
}

async function sendState() {
  const accounts = [];
  for (const { account, inbox } of await inboxes()) {
    let unread = 0;
    try {
      const info = await browser.folders.getFolderInfo(inbox.id ?? inbox);
      unread = info.unreadMessageCount ?? 0;
    } catch {
      try { const info = await browser.folders.getFolderInfo(inbox); unread = info.unreadMessageCount ?? 0; } catch { /* brak */ }
    }
    accounts.push({ id: account.id, name: account.name, unread });
  }
  const flagged = [];
  try {
    for await (const m of iterate(await browser.messages.query({ flagged: true }))) {
      flagged.push(await describe(m, false));
      if (flagged.length >= 25) break;
    }
  } catch { /* stary Thunderbird */ }
  flagged.sort((a, b) => b.date.localeCompare(a.date));
  await post('/mail/state', { accounts, flagged });
}

browser.messages.onUpdated?.addListener(() => scheduleState());
browser.messages.onMoved?.addListener(() => scheduleState());
browser.messages.onDeleted?.addListener(() => scheduleState());
browser.folders.onFolderInfoChanged?.addListener(() => scheduleState());

// ------------------------------------------------------------------ polecenia z Edge Shell

// Id wiadomości w Thunderbirdzie żyją tylko do restartu — wtedy szukamy po Message-ID.
async function resolve(c) {
  if (c.id != null) {
    try {
      const m = await browser.messages.get(c.id);
      if (m && (!c.mid || m.headerMessageId === c.mid)) return m.id;
    } catch { /* nieaktualne id */ }
  }
  if (c.mid) {
    const l = await browser.messages.query({ headerMessageId: c.mid });
    if (l.messages?.length) return l.messages[0].id;
  }
  throw new Error('Tego maila już nie ma w Thunderbirdzie');
}

const escHtml = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

async function reply(id, text) {
  const tab = await browser.compose.beginReply(id, 'replyToSender');
  const d = await browser.compose.getComposeDetails(tab.id);
  if (d.isPlainText) {
    await browser.compose.setComposeDetails(tab.id, { plainTextBody: `${text}\n\n${d.plainTextBody || ''}` });
  } else {
    const p = `<p>${escHtml(text).replace(/\n/g, '<br>')}</p>`;
    const body = d.body || '';
    const withText = /<body[^>]*>/i.test(body) ? body.replace(/<body[^>]*>/i, (m) => m + p) : p + body;
    await browser.compose.setComposeDetails(tab.id, { body: withText });
  }
  await browser.compose.sendMessage(tab.id, { mode: 'sendNow' });
  try { await browser.messages.update(id, { read: true }); } catch { /* bez znaczenia */ }
}

async function search(q) {
  let list;
  try { list = await browser.messages.query({ fullText: q }); } catch { list = null; }
  const seen = new Set();
  const out = [];
  const take = async (l) => {
    for await (const m of iterate(l)) {
      if (seen.has(m.id)) continue;
      seen.add(m.id);
      out.push(await describe(m, false));
      if (out.length >= 40) break;
    }
  };
  if (list) await take(list);
  if (!out.length) {
    try { await take(await browser.messages.query({ subject: q })); } catch { /* brak */ }
    try { await take(await browser.messages.query({ author: q })); } catch { /* brak */ }
  }
  out.sort((a, b) => b.date.localeCompare(a.date));
  return out.slice(0, 8);
}

function vcardField(vcard, name) {
  const m = new RegExp(`^${name}[^:]*:(.*)$`, 'mi').exec(vcard || '');
  return m ? m[1].trim() : '';
}

async function contacts(q) {
  const out = [];
  try {
    const nodes = await browser.contacts.quickSearch(q);
    for (const n of nodes || []) {
      const p = n.properties || {};
      const email = p.PrimaryEmail || vcardField(p.vCard || n.vCard, 'EMAIL');
      const name = p.DisplayName || [p.FirstName, p.LastName].filter(Boolean).join(' ') || vcardField(p.vCard || n.vCard, 'FN');
      if (email && !out.some((c) => c.email === email)) out.push({ name: name || email, email });
      if (out.length >= 6) break;
    }
  } catch { /* brak książki adresowej */ }
  return out;
}

async function unreadInbox() {
  const out = [];
  for (const { inbox } of await inboxes()) {
    let list = null;
    try { list = await browser.messages.query({ folder: inbox, unread: true }); } catch { /* nowsze API */ }
    if (!list) {
      try { list = await browser.messages.query({ folderId: inbox.id, unread: true }); } catch { list = null; }
    }
    if (!list) continue;
    for await (const m of iterate(list)) {
      if (m.junk) continue;
      out.push(m);
      if (out.length >= 60) break;
    }
  }
  out.sort((a, b) => new Date(a.date) - new Date(b.date));
  const described = [];
  for (const m of out.slice(0, 40)) described.push(await describe(m, described.length < 15));
  return described;
}

async function handle(c) {
  let result = { ok: true };
  try {
    switch (c.cmd) {
      case 'open': {
        const id = await resolve(c);
        await browser.messageDisplay.open({ messageId: id, location: 'window', active: true });
        break;
      }
      case 'archive': await browser.messages.archive([await resolve(c)]); break;
      case 'read': await browser.messages.update(await resolve(c), { read: true }); break;
      case 'delete': await browser.messages.delete([await resolve(c)], false); break;
      case 'flag': await browser.messages.update(await resolve(c), { flagged: true }); break;
      case 'unflag': await browser.messages.update(await resolve(c), { flagged: false }); break;
      case 'reply': await reply(await resolve(c), c.text || ''); break;
      case 'compose': await browser.compose.beginNew({ to: c.to ? [c.to] : [], subject: c.subject || '' }); break;
      case 'search': result = { ok: true, items: await search(c.q || '') }; break;
      case 'contacts': result = { ok: true, items: await contacts(c.q || '') }; break;
      case 'inbox': result = { ok: true, items: await unreadInbox() }; break;
      case 'state': await sendState(); break;
      default: result = { ok: false, error: `nieznane polecenie ${c.cmd}` };
    }
  } catch (e) {
    result = { ok: false, error: String(e?.message || e) };
  }
  if (c.reqId) await post('/mail/result', { reqId: c.reqId, ...result });
  if (['archive', 'read', 'delete', 'flag', 'unflag', 'reply'].includes(c.cmd)) scheduleState();
}

let wasConnected = false;
async function poll() {
  try {
    const r = await fetch(url('/mail/commands'), { headers: headers() });
    if (r.ok) {
      if (!wasConnected) { wasConnected = true; scheduleState(); }
      const list = await r.json();
      for (const c of list) handle(c);
    } else {
      wasConnected = false;
    }
  } catch {
    wasConnected = false;
  }
  setTimeout(poll, wasConnected ? 800 : 4000);
}

loadCfg().then(() => {
  poll();
  // Co 5 min odśwież liczniki (np. po zmianach na telefonie).
  setInterval(sendState, 5 * 60 * 1000);
});
