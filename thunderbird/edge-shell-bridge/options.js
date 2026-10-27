const DEF = { port: 7311, token: '' };
const $ = (id) => document.getElementById(id);

async function check() {
  const { port, token } = await browser.storage.local.get(DEF);
  try {
    const r = await fetch(`http://127.0.0.1:${port}/health`, { headers: token ? { Authorization: `Bearer ${token}` } : {} });
    $('st').textContent = r.ok ? '✓ Połączono z Edge Shell' : `Edge Shell odpowiada: ${r.status} (sprawdź token)`;
  } catch {
    $('st').textContent = 'Brak połączenia — czy Edge Shell działa?';
  }
}

browser.storage.local.get(DEF).then((v) => {
  $('port').value = v.port;
  $('token').value = v.token;
  check();
});
$('port').onchange = () => browser.storage.local.set({ port: Number($('port').value) || 7311 }).then(check);
$('token').onchange = () => browser.storage.local.set({ token: $('token').value.trim() }).then(check);
