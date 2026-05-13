use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use tauri::{Emitter, Manager};

const TOOLBAR_HEIGHT: f64 = 38.0;
const BROWSER_BRIDGE_SCRIPT: &str = r#"
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
      if (!isVisible(el) || isJunk(el)) continue;
      pushUnique(out, seen, textOf(el), 140);
    }
    return out;
  }

  function collectLinks(root) {
    var out = [];
    var seen = new Set();
    var nodes = root ? root.querySelectorAll('a[href]') : [];
    for (var i = 0; i < nodes.length && out.length < 8; i++) {
      var el = nodes[i];
      if (!isVisible(el) || isJunk(el)) continue;
      var text = textOf(el);
      var href = normalizeText(el.href || el.getAttribute('href') || '');
      if (!href || href === '#' || href.startsWith('javascript:')) continue;
      var key = (text + '|' + href).toLowerCase();
      if (seen.has(key)) continue;
      seen.add(key);
      out.push({
        text: text.length > 120 ? text.slice(0, 120) + '…' : text,
        url: href.length > 240 ? href.slice(0, 240) + '…' : href
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
      if (!isVisible(el) || isJunk(el)) continue;
      var text = textOf(el) || normalizeText(el.value || el.getAttribute('aria-label') || el.getAttribute('title') || '');
      if (!text) continue;
      var key = (text + '|' + buttonKind(el)).toLowerCase();
      if (seen.has(key)) continue;
      seen.add(key);
      out.push({
        text: text.length > 120 ? text.slice(0, 120) + '…' : text,
        type: buttonKind(el)
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
      if (!isVisible(el) || isJunk(el)) continue;
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

  function candidateText(el) {
    if (!el) return '';
    return normalizeText(
      textOf(el) ||
      el.getAttribute('aria-label') ||
      el.getAttribute('title') ||
      el.getAttribute('value') ||
      el.getAttribute('alt') ||
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

  function currentPageMeta() {
    return {
      url: String(window.location.href || ''),
      title: normalizeText(document.title || '')
    };
  }

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
    return 'a[href],button,[role="button"],[role="link"],[role="tab"],summary,' +
      'input[type="button"],input[type="submit"],input[type="reset"],label,[onclick]';
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

  function selectOptionValue(el, rawValue) {
    var value = normalizeText(rawValue);
    if (!el || String(el.tagName || '').toLowerCase() !== 'select') {
      throw new Error('Target is not a <select> element');
    }
    var options = Array.from(el.options || []);
    var lower = value.toLowerCase();
    var bestIndex = -1;
    var bestScore = 0;
    for (var i = 0; i < options.length; i++) {
      var option = options[i];
      var optionText = normalizeText(option.text || option.label || '');
      var optionValue = normalizeText(option.value || '');
      var score = Math.max(matchScore(value, optionText), matchScore(value, optionValue));
      if (optionValue.toLowerCase() === lower || optionText.toLowerCase() === lower) {
        score += 800;
      }
      if (score > bestScore) {
        bestScore = score;
        bestIndex = i;
      }
    }
    if (bestIndex < 0) {
      throw new Error('No matching option found');
    }
    el.focus();
    el.selectedIndex = bestIndex;
    dispatchInputEvents(el);
  }

  function performClick(el) {
    if (!el) throw new Error('No clickable element found');
    if (typeof el.focus === 'function') el.focus();
    ['mouseover', 'mousedown', 'mouseup'].forEach(function (type) {
      el.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, view: window }));
    });
    if (typeof el.click === 'function') {
      el.click();
    } else {
      el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, view: window }));
    }
  }

  function normalizeKeyName(rawKey) {
    var key = normalizeText(rawKey);
    var lower = key.toLowerCase();
    var map = {
      enter: 'Enter',
      tab: 'Tab',
      escape: 'Escape',
      esc: 'Escape',
      backspace: 'Backspace',
      delete: 'Delete',
      arrowup: 'ArrowUp',
      arrowdown: 'ArrowDown',
      arrowleft: 'ArrowLeft',
      arrowright: 'ArrowRight',
      space: ' ',
      spacebar: ' ',
      pageup: 'PageUp',
      pagedown: 'PageDown',
      home: 'Home',
      end: 'End'
    };
    return map[lower] || key;
  }

  function performKeyPress(el, rawKey) {
    var key = normalizeKeyName(rawKey);
    var target = el || document.activeElement || document.body;
    if (!target) throw new Error('No target available for key press');
    if (typeof target.focus === 'function') target.focus();

    ['keydown', 'keypress', 'keyup'].forEach(function (type) {
      target.dispatchEvent(new KeyboardEvent(type, {
        key: key,
        bubbles: true,
        cancelable: true
      }));
    });

    if (key === 'Enter' && target.form && String(target.tagName || '').toLowerCase() !== 'textarea') {
      if (typeof target.form.requestSubmit === 'function') target.form.requestSubmit();
      else if (typeof target.form.submit === 'function') target.form.submit();
    }

    if ((key === ' ' || key === 'Enter') && isClickable(target) && typeof target.click === 'function') {
      target.click();
    }

    return key;
  }

  function wait(ms) {
    return new Promise(function (resolve) { setTimeout(resolve, ms); });
  }

  async function waitForCondition(action) {
    var timeoutMs = Math.max(200, Number(action.timeoutMs || 3000));
    var start = Date.now();
    var selector = normalizeText(action.selector || '');
    var text = normalizeText(action.text || '');
    while (Date.now() - start <= timeoutMs) {
      if (selector) {
        var matches = Array.from(document.querySelectorAll(String(action.selector)))
          .filter(function (el) { return isVisible(el) && !isJunk(el); });
        if (matches.length) {
          return {
            ok: true,
            action: 'wait_for',
            waitedMs: Date.now() - start,
            matches: matches.length,
            condition: selector,
            selector: selector,
            element: elementSummary(matches[0])
          };
        }
      }
      if (text) {
        var body = normalizeText((document.body && (document.body.innerText || document.body.textContent)) || '');
        if (body.toLowerCase().includes(text.toLowerCase())) {
          return {
            ok: true,
            action: 'wait_for',
            waitedMs: Date.now() - start,
            matches: 1,
            condition: text,
            textFound: text
          };
        }
      }
      await wait(150);
    }
    throw new Error('Timed out waiting for page condition');
  }

  window.__selahBrowserRunAction = async function (requestId, action) {
    try {
      var invoke = window.__TAURI__?.core?.invoke || window.__TAURI_INTERNALS__?.invoke;
      if (!invoke) return;
      var payload = action || {};
      var kind = normalizeText(payload.kind || payload.action).toLowerCase();
      var report = function (result) {
        return invoke('browser_report_action_result', {
          report: {
            requestId: requestId,
            payload: Object.assign({}, currentPageMeta(), result || {})
          }
        });
      };

      if (!kind) {
        await report({ ok: false, error: 'Missing browser action kind' });
        return;
      }

      if (kind === 'click') {
        var clicked = findClickable(payload);
        if (!clicked.element) throw new Error('No matching clickable element found');
        var clickResult = {
          ok: true,
          action: 'click',
          matches: clicked.matches,
          selector: normalizeText(payload.selector || ''),
          textQuery: normalizeText(payload.text || ''),
          hrefContains: normalizeText(payload.hrefContains || ''),
          element: elementSummary(clicked.element)
        };
        var clickReport = report(clickResult);
        performClick(clicked.element);
        await clickReport;
        return;
      }

      if (kind === 'fill') {
        var filled = findField(payload, { selectOnly: false });
        if (!filled.element) throw new Error('No matching field found');
        setFieldValue(filled.element, payload.value || '');
        await report({
          ok: true,
          action: 'fill',
          matches: filled.matches,
          selector: normalizeText(payload.selector || ''),
          labelQuery: normalizeText(payload.label || ''),
          valuePreview: normalizeText(String(payload.value || '')).slice(0, 120),
          element: elementSummary(filled.element)
        });
        return;
      }

      if (kind === 'select_option') {
        var selected = findField(payload, { selectOnly: true });
        if (!selected.element) throw new Error('No matching select field found');
        selectOptionValue(selected.element, payload.value || '');
        await report({
          ok: true,
          action: 'select_option',
          matches: selected.matches,
          selector: normalizeText(payload.selector || ''),
          labelQuery: normalizeText(payload.label || ''),
          valuePreview: normalizeText(String(payload.value || '')).slice(0, 120),
          element: elementSummary(selected.element)
        });
        return;
      }

      if (kind === 'press') {
        var pressTarget = null;
        if (payload.selector) {
          pressTarget = document.querySelector(String(payload.selector));
        }
        if (pressTarget && !isVisible(pressTarget)) pressTarget = null;
        var pressResult = {
          ok: true,
          action: 'press',
          selector: normalizeText(payload.selector || ''),
          key: performKeyPress(pressTarget, payload.key || '')
        };
        if (pressTarget) {
          pressResult.element = elementSummary(pressTarget);
        }
        await report(pressResult);
        return;
      }

      if (kind === 'scroll') {
        var direction = normalizeText(payload.direction || 'down').toLowerCase();
        var amount = Math.max(80, Number(payload.amount || 900));
        if (payload.selector) {
          var scrollTarget = document.querySelector(String(payload.selector));
          if (!scrollTarget || !isVisible(scrollTarget)) throw new Error('No matching element to scroll into view');
          scrollTarget.scrollIntoView({ block: 'center', inline: 'nearest', behavior: 'auto' });
          await wait(60);
          await report({
            ok: true,
            action: 'scroll',
            selector: normalizeText(payload.selector || ''),
            direction: direction,
            element: elementSummary(scrollTarget),
            scrollY: Math.round(window.scrollY || 0)
          });
          return;
        }

        if (direction === 'top') window.scrollTo({ top: 0, behavior: 'auto' });
        else if (direction === 'bottom') window.scrollTo({ top: document.body ? document.body.scrollHeight : 999999, behavior: 'auto' });
        else if (direction === 'up') window.scrollBy({ top: -amount, behavior: 'auto' });
        else window.scrollBy({ top: amount, behavior: 'auto' });

        await wait(60);
        await report({
          ok: true,
          action: 'scroll',
          direction: direction,
          amount: amount,
          scrollY: Math.round(window.scrollY || 0)
        });
        return;
      }

      if (kind === 'wait_for') {
        var waited = await waitForCondition(payload);
        await report(waited);
        return;
      }

      throw new Error('Unsupported browser action: ' + kind);
    } catch (error) {
      try {
        var invoke = window.__TAURI__?.core?.invoke || window.__TAURI_INTERNALS__?.invoke;
        if (!invoke) return;
        await invoke('browser_report_action_result', {
          report: {
            requestId: requestId,
            payload: Object.assign({}, currentPageMeta(), {
              ok: false,
              action: normalizeText(action && (action.kind || action.action) || '').toLowerCase(),
              error: normalizeText(error && (error.message || String(error)) || 'Browser action failed')
            })
          }
        });
      } catch (_) {}
    }
  };

  window.__selahBrowserExtractText = async function (requestId) {
    try {
      var invoke = window.__TAURI__?.core?.invoke || window.__TAURI_INTERNALS__?.invoke;
      if (!invoke) return;
      var doc = document;
      var title = (doc.title || '').trim();
      var root = pickContentRoot(doc);
      var bodyText = collectContent(root);
      if (!bodyText && doc.body) {
        bodyText = textOf(doc.body);
      }
      await invoke('browser_report_page_text', {
        report: {
          requestId: requestId,
          payload: {
            title: title,
            url: String(window.location.href || ''),
            text: bodyText,
            headings: collectHeadings(root),
            links: collectLinks(root),
            buttons: collectButtons(root),
            inputs: collectInputs(root),
            contentSource: root && root.tagName ? String(root.tagName).toLowerCase() : 'document'
          }
        }
      });
    } catch (_) {}
  };
})();
"#;

static PAGE_TEXT_WAITERS: LazyLock<
    Mutex<HashMap<String, tokio::sync::oneshot::Sender<PageTextPayload>>>,
> = LazyLock::new(|| Mutex::new(HashMap::new()));
static BROWSER_ACTION_WAITERS: LazyLock<
    Mutex<HashMap<String, tokio::sync::oneshot::Sender<Value>>>,
> = LazyLock::new(|| Mutex::new(HashMap::new()));
static BROWSER_WINDOW_LABELS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct BrowserWindowInfo {
    pub label: String,
    pub target: String,
    pub url: String,
}

#[derive(Debug, Clone, Default, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserLinkPayload {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, Default, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserButtonPayload {
    #[serde(default)]
    pub text: String,
    #[serde(default, rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Default, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserInputPayload {
    #[serde(default)]
    pub label: String,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub placeholder: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub disabled: bool,
}

#[derive(Debug, Clone, Default, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageTextPayload {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub headings: Vec<String>,
    #[serde(default)]
    pub links: Vec<BrowserLinkPayload>,
    #[serde(default)]
    pub buttons: Vec<BrowserButtonPayload>,
    #[serde(default)]
    pub inputs: Vec<BrowserInputPayload>,
    #[serde(default)]
    pub content_source: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserPageTextReport {
    request_id: String,
    payload: PageTextPayload,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserActionReport {
    request_id: String,
    payload: Value,
}

/// Create a browser-style window with a native toolbar webview + content webview.
/// The toolbar is a local HTML page with back/forward/reload/URL/open-in-browser.
/// The content webview loads the external URL.
pub fn create_browser_window(
    app: &tauri::AppHandle,
    label: &str,
    url: tauri::WebviewUrl,
    title: &str,
    width: f64,
    height: f64,
    init_scripts: &[&str],
) -> Result<(), String> {
    let toolbar_label = format!("{}-tb", label);
    let content_label = format!("{}-ct", label);

    let builder = tauri::window::WindowBuilder::new(app, label)
        .title(title)
        .inner_size(width, height)
        .resizable(true);

    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);

    let window = builder
        .build()
        .map_err(|e| format!("ウィンドウ作成失敗: {}", e))?;
    BROWSER_WINDOW_LABELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(label.to_string());

    // --- Toolbar webview (local HTML) ---
    let toolbar_url = format!(
        "browser-toolbar.html?target={}",
        urlencoding::encode(&content_label)
    );
    let toolbar_builder = tauri::webview::WebviewBuilder::new(
        &toolbar_label,
        tauri::WebviewUrl::App(toolbar_url.into()),
    )
    .auto_resize();

    window
        .add_child(
            toolbar_builder,
            tauri::Position::Logical(tauri::LogicalPosition::new(0.0, 0.0)),
            tauri::Size::Logical(tauri::LogicalSize::new(width, TOOLBAR_HEIGHT)),
        )
        .map_err(|e| format!("ツールバー作成失敗: {}", e))?;

    // --- Content webview ---
    let mut content_builder = tauri::webview::WebviewBuilder::new(&content_label, url)
        .initialization_script(BROWSER_BRIDGE_SCRIPT);
    for script in init_scripts {
        content_builder = content_builder.initialization_script(*script);
    }

    // Emit URL changes to the toolbar
    let app_for_event = app.clone();
    let tb_label_event = toolbar_label.clone();
    content_builder = content_builder.on_page_load(move |_webview, payload| {
        use tauri::webview::PageLoadEvent;
        if matches!(payload.event(), PageLoadEvent::Finished) {
            let url_str = payload.url().to_string();
            let _ = app_for_event.emit_to(
                tauri::EventTarget::AnyLabel {
                    label: tb_label_event.clone(),
                },
                "browser-url-changed",
                &url_str,
            );
        }
    });

    window
        .add_child(
            content_builder,
            tauri::Position::Logical(tauri::LogicalPosition::new(0.0, TOOLBAR_HEIGHT)),
            tauri::Size::Logical(tauri::LogicalSize::new(width, height - TOOLBAR_HEIGHT)),
        )
        .map_err(|e| format!("コンテンツ作成失敗: {}", e))?;

    // --- Handle window resize ---
    let app_resize = app.clone();
    let tb_label_resize = toolbar_label;
    let ct_label_resize = content_label;
    let win_for_scale = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Resized(phys_size) = event {
            let scale = win_for_scale.scale_factor().unwrap_or(1.0);
            let w = phys_size.width as f64 / scale;
            let h = phys_size.height as f64 / scale;

            if let Some(tb) = app_resize.get_webview(&tb_label_resize) {
                let _ = tb.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
                    w,
                    TOOLBAR_HEIGHT,
                )));
            }
            if let Some(ct) = app_resize.get_webview(&ct_label_resize) {
                let _ = ct.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
                    w,
                    (h - TOOLBAR_HEIGHT).max(0.0),
                )));
            }
        }
    });

    Ok(())
}

// ============ Browser Control Commands ============

#[tauri::command]
pub async fn browser_go_back(app: tauri::AppHandle, target: String) -> Result<(), String> {
    let wv = app.get_webview(&target).ok_or("Webview not found")?;
    wv.eval("history.back()").map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_go_forward(app: tauri::AppHandle, target: String) -> Result<(), String> {
    let wv = app.get_webview(&target).ok_or("Webview not found")?;
    wv.eval("history.forward()").map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_reload(app: tauri::AppHandle, target: String) -> Result<(), String> {
    let wv = app.get_webview(&target).ok_or("Webview not found")?;
    wv.eval("location.reload()").map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_get_url(app: tauri::AppHandle, target: String) -> Result<String, String> {
    let wv = app.get_webview(&target).ok_or("Webview not found")?;
    wv.url().map(|u| u.to_string()).map_err(|e| e.to_string())
}

/// Close the browser window that owns `target` (which may be either the window
/// label, the `-ct` content webview label, or the `-tb` toolbar webview label).
/// Removes the label from the registry so subsequent `list_browser_windows`
/// calls reflect reality even before Tauri finishes destroying the window.
pub async fn browser_close(app: tauri::AppHandle, target: String) -> Result<String, String> {
    let label = target
        .strip_suffix("-ct")
        .or_else(|| target.strip_suffix("-tb"))
        .map(|s| s.to_string())
        .unwrap_or_else(|| target.clone());
    let window = app
        .get_window(&label)
        .ok_or_else(|| format!("ウィンドウが見つかりません: {}", label))?;
    BROWSER_WINDOW_LABELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&label);
    window
        .close()
        .map_err(|e| format!("ウィンドウを閉じられませんでした: {}", e))?;
    Ok(label)
}

#[tauri::command]
pub async fn browser_report_page_text(report: BrowserPageTextReport) -> Result<(), String> {
    let tx = PAGE_TEXT_WAITERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&report.request_id)
        .ok_or_else(|| "No pending browser text request".to_string())?;
    let _ = tx.send(report.payload);
    Ok(())
}

#[tauri::command]
pub async fn browser_report_action_result(report: BrowserActionReport) -> Result<(), String> {
    let tx = BROWSER_ACTION_WAITERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&report.request_id)
        .ok_or_else(|| "No pending browser action request".to_string())?;
    let _ = tx.send(report.payload);
    Ok(())
}

pub fn list_browser_windows(app: &tauri::AppHandle) -> Vec<BrowserWindowInfo> {
    let labels: Vec<String> = BROWSER_WINDOW_LABELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .cloned()
        .collect();
    let mut items: Vec<BrowserWindowInfo> = labels
        .into_iter()
        .filter_map(|label| {
            let target = format!("{}-ct", &label);
            let toolbar = format!("{}-tb", &label);
            if app.get_window(&label).is_none() {
                return None;
            }
            if app.get_webview(&target).is_none() || app.get_webview(&toolbar).is_none() {
                return None;
            }
            let url = app
                .get_webview(&target)
                .and_then(|wv| wv.url().ok())
                .map(|u| u.to_string())
                .unwrap_or_default();
            Some(BrowserWindowInfo { label, target, url })
        })
        .collect();
    items.sort_by(|a, b| a.label.cmp(&b.label));
    items
}

pub fn resolve_browser_target(
    app: &tauri::AppHandle,
    requested: Option<&str>,
) -> Result<String, String> {
    if let Some(target) = requested {
        let trimmed = target.trim();
        if trimmed.is_empty() {
            return Err("browser target is empty".into());
        }
        if app.get_webview(trimmed).is_some() {
            return Ok(trimmed.to_string());
        }
        let content = format!("{}-ct", trimmed);
        if app.get_webview(&content).is_some() {
            return Ok(content);
        }
        return Err(format!("Browser target not found: {}", trimmed));
    }
    let items = list_browser_windows(app);
    match items.as_slice() {
        [] => Err("No browser window is open".into()),
        [only] => Ok(only.target.clone()),
        _ => Err("Multiple browser windows are open; list_browser_windows first".into()),
    }
}

pub async fn extract_page_text(
    app: &tauri::AppHandle,
    target: &str,
) -> Result<PageTextPayload, String> {
    let wv = app.get_webview(target).ok_or("Webview not found")?;

    for attempt in 0..5 {
        let request_id = format!("browser-text-{}", uuid::Uuid::new_v4());
        let (tx, rx) = tokio::sync::oneshot::channel();
        PAGE_TEXT_WAITERS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(request_id.clone(), tx);

        let js = format!(
            "(function(){{ if (window.__selahBrowserExtractText) window.__selahBrowserExtractText({}); }})();",
            serde_json::to_string(&request_id).unwrap_or_else(|_| "\"\"".into())
        );

        if let Err(e) = wv.eval(&js) {
            PAGE_TEXT_WAITERS
                .lock()
                .unwrap_or_else(|pe| pe.into_inner())
                .remove(&request_id);
            return Err(e.to_string());
        }

        match tokio::time::timeout(std::time::Duration::from_millis(1200), rx).await {
            Ok(Ok(payload))
                if !payload.url.is_empty()
                    && payload.url != "about:blank"
                    && (!payload.text.trim().is_empty() || attempt >= 2) =>
            {
                return Ok(payload);
            }
            Ok(Ok(_)) | Ok(Err(_)) | Err(_) => {
                PAGE_TEXT_WAITERS
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&request_id);
                if attempt < 4 {
                    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
                    continue;
                }
            }
        }
    }
    Err("Timed out while extracting page text".into())
}

pub async fn run_browser_action(
    app: &tauri::AppHandle,
    target: &str,
    action: &Value,
    timeout_ms: u64,
) -> Result<Value, String> {
    let wv = app.get_webview(target).ok_or("Webview not found")?;
    let request_id = format!("browser-action-{}", uuid::Uuid::new_v4());
    let (tx, rx) = tokio::sync::oneshot::channel();
    BROWSER_ACTION_WAITERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(request_id.clone(), tx);

    let js = format!(
        "(function(){{ if (window.__selahBrowserRunAction) window.__selahBrowserRunAction({}, {}); else window.__TAURI__?.core?.invoke?.('browser_report_action_result', {{ report: {{ requestId: {}, payload: {{ ok: false, error: 'Browser action bridge unavailable' }} }} }}); }})();",
        serde_json::to_string(&request_id).unwrap_or_else(|_| "\"\"".into()),
        serde_json::to_string(action).unwrap_or_else(|_| "{}".into()),
        serde_json::to_string(&request_id).unwrap_or_else(|_| "\"\"".into()),
    );

    if let Err(e) = wv.eval(&js) {
        BROWSER_ACTION_WAITERS
            .lock()
            .unwrap_or_else(|pe| pe.into_inner())
            .remove(&request_id);
        return Err(e.to_string());
    }

    match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms.max(300)), rx).await {
        Ok(Ok(payload)) => Ok(payload),
        Ok(Err(_)) => Err("Browser action channel closed".into()),
        Err(_) => {
            BROWSER_ACTION_WAITERS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&request_id);
            Err(format!(
                "Timed out while waiting for browser action after {} ms",
                timeout_ms.max(300)
            ))
        }
    }
}
