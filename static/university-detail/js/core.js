(function() {
  var t = localStorage.getItem('selah-theme');
  if (t === 'light' || t === 'dark') document.documentElement.setAttribute('data-theme', t);
})();

function escapeHtml(s) { var d = document.createElement('div'); d.textContent = s; return d.innerHTML; }
function decodeHtmlEntities(s) {
  var raw = String(s || '');
  if (!raw || raw.indexOf('&') < 0) return raw;
  var t = document.createElement('textarea');
  t.innerHTML = raw;
  return t.value;
}
function isLunaTransientDetailError(err) {
  var msg = String(err || '');
  return msg.indexOf('読込が一時的に不安定') >= 0
    || msg.indexOf('初回読込が安定していません') >= 0;
}
function delay(ms) {
  return new Promise(function(resolve) { setTimeout(resolve, ms); });
}
function afterFirstPaint(callback, timeout) {
  var run = function() {
    if (typeof requestIdleCallback === 'function') {
      requestIdleCallback(callback, { timeout: timeout || 1200 });
    } else {
      setTimeout(callback, timeout || 120);
    }
  };
  if (typeof requestAnimationFrame === 'function') {
    requestAnimationFrame(function() { requestAnimationFrame(run); });
  } else {
    setTimeout(run, 0);
  }
}
async function invokeLunaDetailWithRetry(command, payload, attempt) {
  var inv = window.__TAURI__?.core?.invoke;
  if (!inv) throw new Error('Tauri IPC が利用できません');
  try {
    return await inv(command, payload);
  } catch (err) {
    if (attempt >= 2 || !isLunaTransientDetailError(err)) throw err;
    await delay(500 * attempt);
    return invokeLunaDetailWithRetry(command, payload, attempt + 1);
  }
}
