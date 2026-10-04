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

      if (kind === 'mouse_click') {
        var mouseClick = performMouseClick(payload);
        await wait(80);
        await report({
          ok: true,
          action: 'mouse_click',
          x: mouseClick.point.x,
          y: mouseClick.point.y,
          element: mouseClick.element
        });
        return;
      }

      if (kind === 'mouse_drag') {
        var drag = await performMouseDrag(payload);
        await wait(80);
        await report({
          ok: true,
          action: 'mouse_drag',
          from: drag.from,
          to: drag.to,
          element: drag.element
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
            viewport: currentPageMeta().viewport,
            text: bodyText,
            headings: collectHeadings(root),
            links: collectLinks(doc),
            buttons: collectButtons(doc),
            inputs: collectInputs(doc),
            contentSource: root && root.tagName ? String(root.tagName).toLowerCase() : 'document'
          }
        }
      });
    } catch (_) {}
  };
})();
