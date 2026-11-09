const $ = id => document.getElementById(id);

chrome.runtime.sendMessage({ type: 'status' }, s => {
  if (!s || !s.ok) {
    $('status').textContent = 'Программа не запущена';
    $('info').textContent = 'Запустите Manga Local — связь появится сама.';
    return;
  }
  $('dot').classList.add('on');
  $('status').textContent = 'Связь с программой есть';
  $('info').textContent = (s.reading ? 'Перевод включён (F8). ' : 'Перевод выключен — нажмите F8. ') +
    'Язык: ' + s.lang + '. ' +
    (s.journal ? 'Журнал для разработчика ведётся.' : 'Чтобы отмечать ошибки, включите в программе «Журнал для разработчика».');
  $('mark').disabled = false;
});

$('mark').onclick = async () => {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  chrome.runtime.sendMessage({ type: 'mark', note: $('note').value, url: tab && tab.url }, r => {
    if (!r || !r.ok) $('done').textContent = 'Программа не ответила';
    else if (!r.journal) $('done').textContent = 'Не записано: журнал в программе выключен';
    else { $('done').textContent = 'Отмечено в журнале ✓'; $('note').value = ''; }
  });
};
