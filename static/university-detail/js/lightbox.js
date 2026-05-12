(function () {
  var overlay = document.getElementById('lightbox-overlay');
  if (!overlay) return;
  var lbImg = document.getElementById('lightbox-img');
  var closeBtn = document.getElementById('lightbox-close');
  var content = document.getElementById('content');

  // ── Zoom / pan state ──
  var scale = 1;
  var tx = 0, ty = 0;
  var MIN_SCALE = 1, MAX_SCALE = 8;
  var dragging = false, lastX = 0, lastY = 0;

  function applyTransform() {
    // Clamp scale
    scale = Math.max(MIN_SCALE, Math.min(MAX_SCALE, scale));
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

  function openLightbox(src, alt) {
    resetTransform();
    lbImg.src = src;
    lbImg.alt = alt || '';
    overlay.classList.add('open');
    document.body.style.overflow = 'hidden';
  }

  function closeLightbox() {
    overlay.classList.remove('open');
    document.body.style.overflow = '';
    resetTransform();
    setTimeout(function () {
      if (!overlay.classList.contains('open')) {
        lbImg.src = '';
        lbImg.alt = '';
      }
    }, 200);
  }

  // ── Mouse wheel zoom ──
  overlay.addEventListener('wheel', function (e) {
    e.preventDefault();
    var delta = e.deltaY < 0 ? 0.15 : -0.15;
    // Zoom toward cursor
    var rect = lbImg.getBoundingClientRect();
    var cx = e.clientX - (rect.left + rect.width / 2);
    var cy = e.clientY - (rect.top + rect.height / 2);
    var prevScale = scale;
    scale = Math.max(MIN_SCALE, Math.min(MAX_SCALE, scale + delta * scale));
    var ratio = scale / prevScale;
    tx = tx * ratio + cx * (1 - ratio);
    ty = ty * ratio + cy * (1 - ratio);
    if (scale === MIN_SCALE) { tx = 0; ty = 0; }
    applyTransform();
  }, { passive: false });

  // ── Double-click to toggle 2× zoom ──
  lbImg.addEventListener('dblclick', function (e) {
    e.stopPropagation();
    if (scale > 1) {
      resetTransform();
    } else {
      scale = 2;
      tx = 0; ty = 0;
      applyTransform();
    }
  });

  // ── Drag to pan when zoomed ──
  lbImg.addEventListener('mousedown', function (e) {
    if (scale <= 1) return;
    e.preventDefault();
    dragging = true;
    lastX = e.clientX;
    lastY = e.clientY;
    lbImg.style.cursor = 'grabbing';
  });
  document.addEventListener('mousemove', function (e) {
    if (!dragging) return;
    tx += e.clientX - lastX;
    ty += e.clientY - lastY;
    lastX = e.clientX;
    lastY = e.clientY;
    applyTransform();
  });
  document.addEventListener('mouseup', function () {
    if (!dragging) return;
    dragging = false;
    lbImg.style.cursor = scale > 1 ? 'grab' : 'default';
  });

  // ── Click overlay background to close (only when not dragging and not zoomed) ──
  overlay.addEventListener('click', function (e) {
    if (e.target === overlay && !dragging) closeLightbox();
  });

  closeBtn.addEventListener('click', closeLightbox);

  // ── Escape key to close ──
  document.addEventListener('keydown', function (e) {
    if (!overlay.classList.contains('open')) return;
    if (e.key === 'Escape') closeLightbox();
    if (e.key === '+' || e.key === '=') { scale = Math.min(MAX_SCALE, scale + 0.3); applyTransform(); }
    if (e.key === '-') { scale = Math.max(MIN_SCALE, scale - 0.3); if (scale === MIN_SCALE) { tx = 0; ty = 0; } applyTransform(); }
    if (e.key === '0') { resetTransform(); }
  });

  // ── Delegate: catch any img click inside #content ──
  document.addEventListener('click', function (e) {
    var t = e.target;
    if (!t || t.tagName !== 'IMG') return;
    var node = t.parentNode;
    var insideContent = false;
    while (node) {
      if (node === content) { insideContent = true; break; }
      var tn = node.tagName;
      if (tn === 'A' || tn === 'BUTTON') return;
      node = node.parentNode;
    }
    if (!insideContent) return;
    if (t.src && t.src !== window.location.href) {
      e.stopPropagation();
      openLightbox(t.src, t.alt);
    }
  });
})();
