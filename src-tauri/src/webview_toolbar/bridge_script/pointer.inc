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
    // A target="_blank" link clicked inside the webview opens a detached/blocked
    // window, so the destination never appears and automation looks like it
    // "did nothing". Force same-tab navigation so the page actually loads in the
    // current webview (the agent keeps the same target and the user sees it).
    // JS window.open() calls are still routed to a Copilot tab via the override.
    try {
      var navAnchor = (el.tagName && String(el.tagName).toLowerCase() === 'a')
        ? el
        : (el.closest ? el.closest('a[href]') : null);
      if (navAnchor) {
        var navTarget = String(navAnchor.getAttribute('target') || '').toLowerCase();
        if (navTarget === '_blank' || navTarget === '_new') {
          navAnchor.setAttribute('target', '_self');
        }
      }
    } catch (e) {}
    try { if (typeof el.focus === 'function') el.focus(); } catch (e) {}
    var rect = el.getBoundingClientRect();
    var cx = Math.round(rect.left + rect.width / 2);
    var cy = Math.round(rect.top + rect.height / 2);
    var opts = { bubbles: true, cancelable: true, view: window, clientX: cx, clientY: cy, button: 0, buttons: 1 };
    var PE = window.PointerEvent;
    var pOpts = Object.assign({ pointerId: 1, pointerType: 'mouse', isPrimary: true }, opts);
    function fireP(type) { if (PE) { try { el.dispatchEvent(new PE(type, pOpts)); } catch (e) {} } }
    function fireM(type, extra) {
      try { el.dispatchEvent(new MouseEvent(type, Object.assign({}, opts, extra || {}))); } catch (e) {}
    }
    // Full pointer + mouse sequence so controls that only listen for pointer
    // events (custom buttons, SPA widgets) also fire, not just legacy mouse ones.
    fireP('pointerover'); fireM('mouseover');
    fireP('pointerenter');
    fireP('pointerdown'); fireM('mousedown');
    fireP('pointerup'); fireM('mouseup', { buttons: 0 });
    if (typeof el.click === 'function') { el.click(); } else { fireM('click', { buttons: 0 }); }
  }

  function viewportPoint(action, prefix) {
    var xKey = prefix ? prefix + 'X' : 'x';
    var yKey = prefix ? prefix + 'Y' : 'y';
    var x = Number(action[xKey] ?? action[(prefix || '') + 'x']);
    var y = Number(action[yKey] ?? action[(prefix || '') + 'y']);
    if (!Number.isFinite(x) || !Number.isFinite(y)) {
      throw new Error('Mouse coordinates are required');
    }
    x = Math.max(0, Math.min(Math.round(x), Math.max(0, window.innerWidth || 0)));
    y = Math.max(0, Math.min(Math.round(y), Math.max(0, window.innerHeight || 0)));
    return { x: x, y: y };
  }

  function dispatchMouse(type, point, button) {
    var el = document.elementFromPoint(point.x, point.y) || document.body || document.documentElement;
    if (!el) throw new Error('No element at mouse coordinates');
    el.dispatchEvent(new MouseEvent(type, {
      bubbles: true,
      cancelable: true,
      view: window,
      clientX: point.x,
      clientY: point.y,
      button: button || 0,
      buttons: type === 'mouseup' ? 0 : 1
    }));
    return el;
  }

  function dispatchPointer(type, point) {
    if (!window.PointerEvent) return;
    var el = document.elementFromPoint(point.x, point.y) || document.body || document.documentElement;
    if (!el) return;
    try {
      el.dispatchEvent(new PointerEvent(type, {
        bubbles: true, cancelable: true, view: window,
        clientX: point.x, clientY: point.y, button: 0,
        buttons: type === 'pointerup' ? 0 : 1,
        pointerId: 1, pointerType: 'mouse', isPrimary: true
      }));
    } catch (e) {}
  }

  function performMouseClick(action) {
    var point = viewportPoint(action, '');
    dispatchPointer('pointerover', point);
    var downEl = dispatchMouse('mouseover', point, 0);
    dispatchMouse('mousemove', point, 0);
    dispatchPointer('pointerdown', point);
    dispatchMouse('mousedown', point, 0);
    dispatchPointer('pointerup', point);
    dispatchMouse('mouseup', point, 0);
    dispatchMouse('click', point, 0);
    return { point: point, element: elementSummary(downEl) };
  }

  async function performMouseDrag(action) {
    var from = viewportPoint(action, 'from');
    var to = viewportPoint(action, 'to');
    var steps = Math.max(2, Math.min(Number(action.steps || 8), 24));
    dispatchPointer('pointerover', from);
    var startEl = dispatchMouse('mouseover', from, 0);
    dispatchMouse('mousemove', from, 0);
    dispatchPointer('pointerdown', from);
    dispatchMouse('mousedown', from, 0);
    for (var i = 1; i <= steps; i++) {
      var point = {
        x: Math.round(from.x + (to.x - from.x) * i / steps),
        y: Math.round(from.y + (to.y - from.y) * i / steps)
      };
      dispatchPointer('pointermove', point);
      dispatchMouse('mousemove', point, 0);
      await wait(16);
    }
    dispatchPointer('pointerup', to);
    dispatchMouse('mouseup', to, 0);
    return { from: from, to: to, element: elementSummary(startEl) };
  }

