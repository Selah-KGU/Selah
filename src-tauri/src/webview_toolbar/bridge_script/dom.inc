
(function () {
  if (window.__selahBrowserBridgeInstalled) return;
  window.__selahBrowserBridgeInstalled = true;

  function normalizeText(value) {
    return String(value || '')
      .replace(/\u00A0/g, ' ')
      .replace(/[ \t]+\n/g, '\n')
      .replace(/\n[ \t]+/g, '\n')
      .replace(/[ \t]{2,}/g, ' ')
      .replace(/\n{3,}/g, '\n\n')
      .trim();
  }

  function textOf(el) {
    if (!el) return '';
    return normalizeText(el.innerText || el.textContent || '');
  }

  function isVisible(el) {
    if (!el || !el.isConnected) return false;
    if (el.hidden || el.getAttribute('aria-hidden') === 'true') return false;
    try {
      var style = window.getComputedStyle(el);
      if (!style) return true;
      if (style.display === 'none' || style.visibility === 'hidden') return false;
      if (parseFloat(style.opacity || '1') === 0) return false;
    } catch (_) {}
    var rect = typeof el.getBoundingClientRect === 'function' ? el.getBoundingClientRect() : null;
    if (!rect) return true;
    return rect.width > 0 || rect.height > 0;
  }

  function isJunk(el) {
    if (!el || !(el instanceof Element)) return false;
    return !!el.closest(
      'script,style,noscript,template,svg,canvas,iframe,nav,header,footer,aside,' +
      '[role="navigation"],[role="banner"],[role="contentinfo"],[role="dialog"],[aria-modal="true"],' +
      '.cookie,.cookies,.consent,.ads,.ad,.advertisement,.breadcrumb,.sidebar,.drawer,.modal,.popup'
    );
  }

  function isControlJunk(el) {
    if (!el || !(el instanceof Element)) return false;
    return !!el.closest(
      'script,style,noscript,template,iframe,[role="dialog"],[aria-modal="true"],' +
      '.cookie,.cookies,.consent,.ads,.ad,.advertisement,.modal,.popup'
    );
  }

  function pushUnique(items, seen, raw, maxChars) {
    var value = normalizeText(raw);
    if (!value) return;
    if (maxChars && value.length > maxChars) value = value.slice(0, maxChars) + '…';
    var key = value.toLowerCase();
    if (seen.has(key)) return;
    seen.add(key);
    items.push(value);
  }

  function scoreRoot(el) {
    if (!el || !isVisible(el) || isJunk(el)) return -1;
    var text = textOf(el);
    if (text.length < 80) return -1;
    var blocks = el.querySelectorAll('p,li,tr').length;
    var headings = el.querySelectorAll('h1,h2,h3').length;
    var controls = el.querySelectorAll('button,input,textarea,select').length;
    return Math.min(text.length, 7000) + blocks * 40 + headings * 120 + controls * 25;
  }

  function pickContentRoot(doc) {
    var preferred = [
      'main',
      'article',
      '[role="main"]',
      '.main',
      '#main',
      '.content',
      '#content',
      '.article',
      '.post',
      '.entry',
      'form'
    ];
    for (var i = 0; i < preferred.length; i++) {
      var candidate = doc.querySelector(preferred[i]);
      if (!candidate || !isVisible(candidate) || isJunk(candidate)) continue;
      if (textOf(candidate).length >= 120 || candidate.querySelector('input,textarea,select,button')) {
        return candidate;
      }
    }

    var candidates = Array.from(doc.querySelectorAll('main,article,section,form,div')).slice(0, 500);
    var best = doc.body || doc.documentElement;
    var bestScore = scoreRoot(best);
    for (var j = 0; j < candidates.length; j++) {
      var el = candidates[j];
      var score = scoreRoot(el);
      if (score > bestScore) {
        best = el;
        bestScore = score;
      }
    }
    return best || doc.body || doc.documentElement;
  }

  function collectHeadings(root) {
    var out = [];
    var seen = new Set();
    var nodes = root ? root.querySelectorAll('h1,h2,h3,h4,h5,h6') : [];
    for (var i = 0; i < nodes.length && out.length < 10; i++) {
      var el = nodes[i];
      if (!isVisible(el) || isControlJunk(el)) continue;
      pushUnique(out, seen, textOf(el), 140);
    }
    return out;
  }

  function collectLinks(root) {
    var out = [];
    var seen = new Set();
    var nodes = root ? root.querySelectorAll('a[href]') : [];
    for (var i = 0; i < nodes.length && out.length < 30; i++) {
      var el = nodes[i];
      if (!isVisible(el) || isControlJunk(el)) continue;
      var text = candidateText(el);
      var href = normalizeText(el.href || el.getAttribute('href') || '');
      if (!href || href === '#' || href.startsWith('javascript:')) continue;
      var key = (text + '|' + href).toLowerCase();
      if (seen.has(key)) continue;
      seen.add(key);
      out.push({
        text: text.length > 120 ? text.slice(0, 120) + '…' : text,
        url: href.length > 240 ? href.slice(0, 240) + '…' : href,
        rect: elementRect(el)
      });
    }
    return out;
  }

  function buttonKind(el) {
    var kind = el.getAttribute('type') || el.getAttribute('role') || el.tagName || '';
    return normalizeText(kind).toLowerCase();
  }

  function collectButtons(root) {
    var out = [];
    var seen = new Set();
    var nodes = root
      ? root.querySelectorAll('button,[role="button"],input[type="button"],input[type="submit"],input[type="reset"]')
      : [];
    for (var i = 0; i < nodes.length && out.length < 10; i++) {
      var el = nodes[i];
      if (!isVisible(el) || isControlJunk(el)) continue;
      var text = candidateText(el);
      if (!text) continue;
      var key = (text + '|' + buttonKind(el)).toLowerCase();
      if (seen.has(key)) continue;
      seen.add(key);
      out.push({
        text: text.length > 120 ? text.slice(0, 120) + '…' : text,
        type: buttonKind(el),
        rect: elementRect(el)
      });
    }
    return out;
  }

  function findLabelText(el) {
    if (!el) return '';
    var parentLabel = el.closest('label');
    if (parentLabel) {
      var parentText = textOf(parentLabel);
      if (parentText) return parentText;
    }
    var id = el.id || '';
    if (id && window.CSS && typeof window.CSS.escape === 'function') {
      var label = document.querySelector('label[for="' + window.CSS.escape(id) + '"]');
      if (label) {
        var labelText = textOf(label);
        if (labelText) return labelText;
      }
    }
    return normalizeText(
      el.getAttribute('aria-label') ||
      el.getAttribute('placeholder') ||
      el.getAttribute('name') ||
      ''
    );
  }

  function collectInputs(root) {
    var out = [];
    var seen = new Set();
    var nodes = root ? root.querySelectorAll('input,textarea,select') : [];
    for (var i = 0; i < nodes.length && out.length < 10; i++) {
      var el = nodes[i];
      if (!isVisible(el) || isControlJunk(el)) continue;
      var tag = (el.tagName || '').toLowerCase();
      var type = normalizeText(el.getAttribute('type') || tag).toLowerCase();
      if (type === 'hidden') continue;
      var label = findLabelText(el);
      var name = normalizeText(el.getAttribute('name') || '');
      var placeholder = normalizeText(el.getAttribute('placeholder') || '');
      var value = normalizeText(el.value || '');
      var key = (label + '|' + name + '|' + type).toLowerCase();
      if (!label && !name && !placeholder && !value) continue;
      if (seen.has(key)) continue;
      seen.add(key);
      out.push({
        label: label.length > 120 ? label.slice(0, 120) + '…' : label,
        type: type,
        name: name.length > 80 ? name.slice(0, 80) + '…' : name,
        placeholder: placeholder.length > 120 ? placeholder.slice(0, 120) + '…' : placeholder,
        value: value.length > 120 ? value.slice(0, 120) + '…' : value,
        rect: elementRect(el),
        required: !!el.required,
        disabled: !!el.disabled
      });
    }
    return out;
  }

  function collectContent(root) {
    var blocks = [];
    var seen = new Set();
    var totalChars = 0;
    var nodes = root ? root.querySelectorAll('h1,h2,h3,h4,h5,h6,p,li,blockquote,pre,tr') : [];

    function pushLine(line) {
      var value = normalizeText(line);
      if (!value) return;
      var key = value.toLowerCase();
      if (seen.has(key)) return;
      if (blocks.length > 0 && totalChars + value.length > 12000) return;
      seen.add(key);
      blocks.push(value);
      totalChars += value.length + 1;
    }

    for (var i = 0; i < nodes.length; i++) {
      if (totalChars >= 12000) break;
      var el = nodes[i];
      if (!isVisible(el) || isJunk(el)) continue;
      var tag = (el.tagName || '').toLowerCase();
      if (tag === 'tr') {
        var cells = Array.from(el.querySelectorAll('th,td'))
          .map(function (cell) { return textOf(cell); })
          .filter(Boolean);
        if (cells.length) pushLine(cells.join(' | '));
        continue;
      }
      var text = textOf(el);
      if (!text) continue;
      if (/^h[1-6]$/.test(tag)) {
        var level = Math.min(Math.max(parseInt(tag.slice(1), 10) || 2, 1), 4);
        pushLine(Array(level + 1).join('#') + ' ' + text);
      } else if (tag === 'li') {
        pushLine('- ' + text);
      } else if (tag === 'blockquote') {
        pushLine('> ' + text);
      } else {
        pushLine(text);
      }
    }

    if (!blocks.length) {
      return textOf(root).slice(0, 12000);
    }
    return blocks.join('\n');
  }

