// Talks to Manga Local on this computer (127.0.0.1:47811): passes on where the
// pictures are and the pictures the program asks for, and the reader's
// «здесь ошибка». Nothing goes anywhere else.
const BASE = 'http://127.0.0.1:47811';
const busy = new Set();
// Pictures the site would not give (some sites check where the request comes
// from): not asked again, the program reads the screen for them.
const failed = new Set();

async function post(path, body, type) {
  const r = await fetch(BASE + path, { method: 'POST', body, headers: type ? { 'Content-Type': type } : {} });
  if (!r.ok) throw new Error('Manga Local: ' + r.status);
  return r.json();
}

async function hello() {
  try {
    const r = await fetch(BASE + '/hello', { cache: 'no-store' });
    const j = await r.json();
    badge(true);
    return { ok: true, ...j };
  } catch (e) {
    badge(false);
    return { ok: false };
  }
}

function badge(on) {
  chrome.action.setBadgeText({ text: on ? 'ON' : '' });
  chrome.action.setBadgeBackgroundColor({ color: on ? '#e500e8' : '#666666' });
}

async function onPage(view, tab) {
  if (!tab.active) return;
  const w = await chrome.windows.get(tab.windowId);
  if (!w.focused) return;
  let answer;
  try {
    answer = await post('/page', JSON.stringify(view), 'application/json');
    badge(true);
  } catch (e) {
    badge(false); // the program is not running
    return;
  }
  const all = [...view.images, ...view.ahead];
  for (const id of answer.want || []) {
    const img = all.find(i => i.id === id);
    if (!img || busy.has(id) || failed.has(img.src)) continue;
    busy.add(id);
    try {
      // The browser usually has it already: taken from its cache.
      const resp = await fetch(img.src, { credentials: 'include' });
      if (!resp.ok) throw new Error(resp.status);
      const blob = await resp.blob();
      if (blob.size > 60e6) throw new Error('too big');
      await post('/image?id=' + encodeURIComponent(id), blob);
    } catch (e) {
      failed.add(img.src);
    } finally {
      busy.delete(id);
    }
  }
}

async function mark(note, url) {
  try {
    const j = await post('/mark', JSON.stringify({ note: note || '', url: url || '' }), 'application/json');
    return { ok: true, journal: j.journal };
  } catch (e) {
    return { ok: false };
  }
}

chrome.runtime.onMessage.addListener((msg, sender, respond) => {
  if (msg.type === 'page' && sender.tab) {
    onPage(msg.view, sender.tab).catch(() => {});
  } else if (msg.type === 'status') {
    hello().then(respond);
    return true;
  } else if (msg.type === 'mark') {
    mark(msg.note, msg.url).then(respond);
    return true;
  }
});

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({ id: 'mark', title: 'Manga Local: здесь ошибка перевода', contexts: ['page', 'image'] });
  hello();
});
chrome.runtime.onStartup.addListener(hello);

chrome.contextMenus.onClicked.addListener((info, tab) => {
  if (info.menuItemId === 'mark') mark('', tab && tab.url);
});
