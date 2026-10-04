  function candidateText(el) {
    if (!el) return '';
    var imgAlt = '';
    try {
      var img = el.querySelector && el.querySelector('img[alt]');
      imgAlt = img ? img.getAttribute('alt') || '' : '';
    } catch (_) {}
    return normalizeText(
      textOf(el) ||
      el.getAttribute('aria-label') ||
      el.getAttribute('title') ||
      el.getAttribute('value') ||
      el.getAttribute('alt') ||
      imgAlt ||
      ''
    );
  }

  function matchScore(query, text) {
    var q = normalizeText(query).toLowerCase();
    var t = normalizeText(text).toLowerCase();
    if (!q || !t) return 0;
    if (t === q) return 1200;
    if (t.startsWith(q)) return 900 - Math.min(240, t.length - q.length);
    if (t.includes(q)) return 700 - Math.min(220, t.length - q.length);
    if (q.includes(t) && t.length >= 2) return 520 - Math.min(180, q.length - t.length);
    return 0;
  }

  function dedupeElements(items) {
    var seen = new Set();
    var out = [];
    for (var i = 0; i < items.length; i++) {
      var el = items[i];
      if (!el || seen.has(el)) continue;
      seen.add(el);
      out.push(el);
    }
    return out;
  }

  function pickByIndex(items, index) {
    if (!items.length) return null;
    var safe = Math.max(0, Math.min(Number(index || 0), items.length - 1));
    return items[safe];
  }

  function elementRect(el) {
    if (!el || typeof el.getBoundingClientRect !== 'function') return null;
    var rect = el.getBoundingClientRect();
    if (!rect || rect.width <= 0 || rect.height <= 0) return null;
    return {
      x: Math.round(rect.left),
      y: Math.round(rect.top),
      width: Math.round(rect.width),
      height: Math.round(rect.height),
      centerX: Math.round(rect.left + rect.width / 2),
      centerY: Math.round(rect.top + rect.height / 2)
    };
  }

  function currentPageMeta() {
    return {
      url: String(window.location.href || ''),
      title: normalizeText(document.title || ''),
      viewport: {
        width: Math.round(window.innerWidth || 0),
        height: Math.round(window.innerHeight || 0),
        scrollX: Math.round(window.scrollX || 0),
        scrollY: Math.round(window.scrollY || 0),
        scrollHeight: Math.round(document.documentElement?.scrollHeight || document.body?.scrollHeight || 0)
      }
    };
  }

  function coreInvoke() {
    return window.__TAURI__?.core?.invoke || window.__TAURI_INTERNALS__?.invoke || null;
  }

  function openPopupThroughApp(rawUrl, title) {
    var invoke = coreInvoke();
    if (!invoke || !rawUrl) return false;
    var resolved = null;
    try {
      resolved = new URL(String(rawUrl || ''), window.location.href);
    } catch (_) {
      return false;
    }
    if (resolved.protocol !== 'http:' && resolved.protocol !== 'https:') return false;
    invoke('open_external_url', {
      url: resolved.toString(),
      title: normalizeText(title || resolved.host || resolved.toString())
    }).catch(function (error) {
      console.warn('open_external_url failed for popup:', error);
    });
    return true;
  }

  try {
    var originalWindowOpen = typeof window.open === 'function' ? window.open.bind(window) : null;
    window.open = function (rawUrl, target, features) {
      if (rawUrl && openPopupThroughApp(rawUrl, document.title || '')) {
        return null;
      }
      return originalWindowOpen ? originalWindowOpen(rawUrl, target, features) : null;
    };
  } catch (_) {}

  function elementSummary(el) {
    if (!el) return {};
    return {
      tag: String(el.tagName || '').toLowerCase(),
      text: candidateText(el).slice(0, 160),
      name: normalizeText(el.getAttribute('name') || '').slice(0, 80),
      type: normalizeText(el.getAttribute('type') || el.getAttribute('role') || '').slice(0, 40),
      href: normalizeText(el.href || el.getAttribute('href') || '').slice(0, 240)
    };
  }

  function clickableSelector() {
    return 'a[href],button,[role="button"],[role="link"],[role="tab"],' +
      '[role="menuitem"],[role="menuitemcheckbox"],[role="menuitemradio"],' +
      '[role="option"],[role="checkbox"],[role="radio"],[role="switch"],[role="treeitem"],' +
      'summary,input[type="button"],input[type="submit"],input[type="reset"],' +
      'input[type="checkbox"],input[type="radio"],label,[onclick],[tabindex]:not([tabindex="-1"])';
  }

  function isClickable(el) {
    if (!el || !isVisible(el) || isJunk(el)) return false;
    if (el.matches && el.matches(clickableSelector())) return true;
    var tag = String(el.tagName || '').toLowerCase();
    return tag === 'a' || tag === 'button';
  }

  function closestClickable(el) {
    if (!el || !el.closest) return null;
    return el.closest(clickableSelector());
  }

  function findClickable(action) {
    var index = Number(action.index || 0);
    if (action.selector) {
      var direct = Array.from(document.querySelectorAll(String(action.selector)))
        .map(function (el) { return closestClickable(el) || el; })
        .filter(function (el) { return isClickable(el); });
      direct = dedupeElements(direct);
      return { element: pickByIndex(direct, index), matches: direct.length };
    }

    var textQuery = normalizeText(action.text || '');
    var hrefQuery = normalizeText(action.hrefContains || '').toLowerCase();
    var ranked = Array.from(document.querySelectorAll(clickableSelector()))
      .filter(function (el) { return isClickable(el); })
      .map(function (el) {
        var score = 0;
        if (textQuery) score += matchScore(textQuery, candidateText(el));
        if (hrefQuery) {
          var href = normalizeText(el.href || el.getAttribute('href') || '').toLowerCase();
          if (href.includes(hrefQuery)) score += href === hrefQuery ? 1300 : 850;
        }
        return { el: el, score: score };
      })
      .filter(function (item) { return item.score > 0; })
      .sort(function (a, b) { return b.score - a.score; });

    var matches = ranked.map(function (item) { return item.el; });
    return { element: pickByIndex(matches, index), matches: matches.length };
  }

  function fieldSelector() {
    return 'input:not([type="hidden"]),textarea,select,[contenteditable="true"]';
  }

  function isFillable(el) {
    if (!el || !isVisible(el) || isJunk(el)) return false;
    if (el.matches && el.matches(fieldSelector())) return true;
    return !!el.isContentEditable;
  }

  function fieldSummaryText(el) {
    return normalizeText([
      findLabelText(el),
      el.getAttribute('name') || '',
      el.getAttribute('placeholder') || '',
      el.getAttribute('aria-label') || '',
      el.getAttribute('title') || ''
    ].filter(Boolean).join(' | '));
  }

  function findField(action, options) {
    var index = Number(action.index || 0);
    var allowSelectOnly = !!(options && options.selectOnly);
    if (action.selector) {
      var direct = Array.from(document.querySelectorAll(String(action.selector)))
        .filter(function (el) { return isFillable(el); });
      if (allowSelectOnly) {
        direct = direct.filter(function (el) { return String(el.tagName || '').toLowerCase() === 'select'; });
      }
      return { element: pickByIndex(direct, index), matches: direct.length };
    }

    var labelQuery = normalizeText(action.label || '');
    var ranked = Array.from(document.querySelectorAll(fieldSelector()))
      .filter(function (el) { return isFillable(el); })
      .filter(function (el) {
        return !allowSelectOnly || String(el.tagName || '').toLowerCase() === 'select';
      })
      .map(function (el) {
        return { el: el, score: matchScore(labelQuery, fieldSummaryText(el)) };
      })
      .filter(function (item) { return labelQuery ? item.score > 0 : true; })
      .sort(function (a, b) { return b.score - a.score; });

    var matches = ranked.map(function (item) { return item.el; });
    return { element: pickByIndex(matches, index), matches: matches.length };
  }

  function setNativeValue(el, value) {
    var proto = null;
    if (window.HTMLInputElement && el instanceof window.HTMLInputElement) {
      proto = window.HTMLInputElement.prototype;
    } else if (window.HTMLTextAreaElement && el instanceof window.HTMLTextAreaElement) {
      proto = window.HTMLTextAreaElement.prototype;
    }
    var desc = proto && Object.getOwnPropertyDescriptor(proto, 'value');
    if (desc && typeof desc.set === 'function') {
      desc.set.call(el, value);
    } else {
      el.value = value;
    }
  }

  function dispatchInputEvents(el) {
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  }

  function setFieldValue(el, rawValue) {
    var value = String(rawValue == null ? '' : rawValue);
    if (el.isContentEditable) {
      el.focus();
      el.textContent = value;
      dispatchInputEvents(el);
      return;
    }

    var tag = String(el.tagName || '').toLowerCase();
    var type = normalizeText(el.getAttribute('type') || '').toLowerCase();
    el.focus();

    if (type === 'checkbox' || type === 'radio') {
      var truthy = ['true', '1', 'yes', 'on', 'checked', '选中', '勾选', 'はい'];
      var shouldCheck = truthy.indexOf(value.trim().toLowerCase()) >= 0;
      el.checked = shouldCheck;
      dispatchInputEvents(el);
      return;
    }

    if (tag === 'select') {
      selectOptionValue(el, value);
      return;
    }

    setNativeValue(el, value);
    dispatchInputEvents(el);
  }

