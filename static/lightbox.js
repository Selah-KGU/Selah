/**
 * Shared lightbox — auto-injects its own HTML & CSS.
 * Works in both university-detail.html and markdown-reader.html.
 */
(function () {
  // ── Inject CSS ──
  var style = document.createElement('style');
  style.textContent = [
    '#kwic-lb{display:none;position:fixed;inset:0;z-index:9000;',
    'background:rgba(0,0,0,.82);align-items:center;justify-content:center;',
    'padding:24px;backdrop-filter:blur(8px);-webkit-backdrop-filter:blur(8px);',
    'cursor:zoom-out;}',
    '#kwic-lb.open{display:flex;animation:kwic-lb-in .18s ease;}',
    '@keyframes kwic-lb-in{from{opacity:0}to{opacity:1}}',
    '#kwic-lb img{max-width:calc(100vw - 64px);max-height:calc(100vh - 64px);',
    'width:auto;height:auto;border-radius:10px;',
    'box-shadow:0 32px 96px rgba(0,0,0,.6);object-fit:contain;display:block;',
    'transform-origin:center center;will-change:transform;',
    'user-select:none;-webkit-user-drag:none;',
    'animation:kwic-lb-zoom .18s cubic-bezier(.2,.8,.2,1);}',
    '@keyframes kwic-lb-zoom{from{transform:scale(.88);opacity:0}to{transform:scale(1);opacity:1}}',
    '#kwic-lb .kwic-lb-close{position:absolute;top:14px;right:14px;',
    'width:32px;height:32px;border-radius:50%;',
    'background:rgba(255,255,255,.15);border:none;color:#fff;font-size:16px;',
    'display:flex;align-items:center;justify-content:center;cursor:pointer;',
    'transition:background .15s;line-height:1;-webkit-app-region:no-drag;}',
    '#kwic-lb .kwic-lb-close:hover{background:rgba(255,255,255,.28);}',
  ].join('');
  document.head.appendChild(style);

  // ── Inject HTML ──
  var overlay = document.createElement('div');
  overlay.id = 'kwic-lb';
  overlay.setAttribute('role', 'dialog');
  overlay.setAttribute('aria-modal', 'true');
  overlay.setAttribute('aria-label', '画像プレビュー');
  overlay.innerHTML = '<button class="kwic-lb-close" type="button" aria-label="閉じる">\u2715</button><img src="" alt="">';
  document.body.appendChild(overlay);

  var lbImg = overlay.querySelector('img');
  var closeBtn = overlay.querySelector('.kwic-lb-close');

  // ── State ──
  var scale = 1, tx = 0, ty = 0;
  var MIN = 1, MAX = 8;
  var dragging = false, lastX = 0, lastY = 0;

  // Wheel blocker used while lightbox is open (prevents page scroll)
  function blockWheel(e) { e.preventDefault(); }

  function applyTransform() {
    scale = Math.max(MIN, Math.min(MAX, scale));
    lbImg.style.transform = 'translate(' + tx + 'px,' + ty + 'px) scale(' + scale + ')';
    lbImg.style.cursor = scale > 1 ? 'grab' : 'default';
    overlay.style.cursor = scale > 1 ? 'default' : 'zoom-out';
  }

  function resetTransform() {
    scale = 1; tx = 0; ty = 0;
    lbImg.style.transform = '';
    lbImg.style.cursor = '';
    overlay.style.cursor = '';
  }

  function open(src, alt) {
    resetTransform();
    lbImg.src = src;
    lbImg.alt = alt || '';
    overlay.classList.add('open');
    // Block all scroll while lightbox is visible
    document.addEventListener('wheel', blockWheel, { passive: false });
  }

  function close() {
    overlay.classList.remove('open');
    document.removeEventListener('wheel', blockWheel);
    resetTransform();
    setTimeout(function () {
      if (!overlay.classList.contains('open')) { lbImg.src = ''; lbImg.alt = ''; }
    }, 200);
  }

  // ── Mouse-wheel zoom (sensitivity: 0.08 per notch) ──
  overlay.addEventListener('wheel', function (e) {
    e.preventDefault();
    var dir = e.deltaY < 0 ? 1 : -1;
    var step = 0.02 * scale * dir;
    var rect = lbImg.getBoundingClientRect();
    var cx = e.clientX - (rect.left + rect.width / 2);
    var cy = e.clientY - (rect.top + rect.height / 2);
    var prev = scale;
    scale = Math.max(MIN, Math.min(MAX, scale + step));
    var ratio = scale / prev;
    tx = tx * ratio + cx * (1 - ratio);
    ty = ty * ratio + cy * (1 - ratio);
    if (scale === MIN) { tx = 0; ty = 0; }
    applyTransform();
  }, { passive: false });

  // ── Double-click: toggle 2× ──
  lbImg.addEventListener('dblclick', function (e) {
    e.stopPropagation();
    if (scale > 1) { resetTransform(); } else { scale = 2; tx = 0; ty = 0; applyTransform(); }
  });

  // ── Drag to pan ──
  lbImg.addEventListener('mousedown', function (e) {
    if (scale <= 1) return;
    e.preventDefault();
    dragging = true; lastX = e.clientX; lastY = e.clientY;
    lbImg.style.cursor = 'grabbing';
  });
  document.addEventListener('mousemove', function (e) {
    if (!dragging) return;
    tx += e.clientX - lastX; ty += e.clientY - lastY;
    lastX = e.clientX; lastY = e.clientY;
    applyTransform();
  });
  document.addEventListener('mouseup', function () {
    if (!dragging) return;
    dragging = false;
    lbImg.style.cursor = scale > 1 ? 'grab' : 'default';
  });

  // ── Click background to close ──
  overlay.addEventListener('click', function (e) {
    if (e.target === overlay && !dragging) close();
  });
  closeBtn.addEventListener('click', close);

  // ── Keyboard ──
  document.addEventListener('keydown', function (e) {
    if (!overlay.classList.contains('open')) return;
    if (e.key === 'Escape') { close(); return; }
    if (e.key === '+' || e.key === '=') { scale = Math.min(MAX, scale + 0.3); applyTransform(); }
    if (e.key === '-') { scale = Math.max(MIN, scale - 0.3); if (scale === MIN) { tx = 0; ty = 0; } applyTransform(); }
    if (e.key === '0') { resetTransform(); }
  });

  // ── Delegate: any img click not inside a / button ──
  document.addEventListener('click', function (e) {
    var t = e.target;
    if (!t || t.tagName !== 'IMG' || t === lbImg) return;
    var node = t.parentNode;
    while (node && node !== document.body) {
      var tn = (node.tagName || '').toUpperCase();
      if (tn === 'A' || tn === 'BUTTON') return;
      node = node.parentNode;
    }
    if (t.src && t.src !== window.location.href) {
      e.stopPropagation();
      open(t.src, t.alt);
    }
  });
})();
