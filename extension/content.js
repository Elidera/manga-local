// Tells the extension where the page's pictures are on screen (after every
// scroll, resize and loaded picture). Reads nothing else from the page.
(() => {
  const MIN = 150; // smaller pictures are icons and avatars, not pages
  let timer = null;

  function idOf(src) {
    let h = 2166136261 >>> 0;
    for (let i = 0; i < src.length; i++) {
      h ^= src.charCodeAt(i);
      h = Math.imul(h, 16777619) >>> 0;
    }
    return h.toString(16) + '-' + src.length;
  }

  function describe(img, r) {
    const src = img.currentSrc || img.src;
    return { id: idOf(src), src, rect: [r.left, r.top, r.width, r.height], natural: [img.naturalWidth, img.naturalHeight] };
  }

  function collect() {
    const vw = innerWidth, vh = innerHeight;
    const visible = [], below = [];
    for (const img of document.images) {
      const src = img.currentSrc || img.src;
      if (!src || src.startsWith('data:') && src.length < 2000) continue;
      const r = img.getBoundingClientRect();
      if (r.width < MIN || r.height < 60) continue;
      const st = getComputedStyle(img);
      if (st.visibility === 'hidden' || st.display === 'none' || parseFloat(st.opacity) < 0.1) continue;
      if (r.bottom > 0 && r.top < vh && r.right > 0 && r.left < vw) {
        // Loading pictures too (natural 0×0): the program then reads the screen.
        visible.push(describe(img, r));
      } else if (r.top >= vh && img.complete && img.naturalWidth >= MIN) {
        below.push([r.top, describe(img, r)]);
      }
    }
    below.sort((a, b) => a[0] - b[0]);
    return {
      url: location.href,
      title: document.title,
      dpr: devicePixelRatio,
      inner: [vw, vh],
      images: visible,
      ahead: below.slice(0, 3).map(x => x[1]),
    };
  }

  function send() {
    if (document.visibilityState !== 'visible') return;
    try {
      chrome.runtime.sendMessage({ type: 'page', view: collect() }).catch(() => {});
    } catch (e) {
      // The extension was reloaded: this old copy of the script stops.
      clearInterval(beat);
    }
  }

  function soon() {
    clearTimeout(timer);
    timer = setTimeout(send, 250);
  }

  // capture: scrolling inside the reader's own scroll box counts too.
  addEventListener('scroll', soon, { passive: true, capture: true });
  addEventListener('resize', soon);
  document.addEventListener('visibilitychange', soon);
  document.addEventListener('load', e => { if (e.target && e.target.tagName === 'IMG') soon(); }, true);
  const beat = setInterval(soon, 5000);
  soon();
})();
