// Download helpers
async function downloadMaterial(idnumber, file, btn, materialTitle) {
  var invoke = window.__TAURI__?.core?.invoke; if (!invoke) return;
  var orig = btn.innerHTML; btn.innerHTML = '<span>\u30c0\u30a6\u30f3\u30ed\u30fc\u30c9\u4e2d...</span>'; btn.disabled = true;
  try {
    var path = await invoke('luna_download_material', { idnumber: idnumber, fileName: file.file_name, objectName: file.object_name, resourceId: file.resource_id, fileType: file.file_type || '0', materialId: file.material_id || null, displayName: file.display_name || null, endDate: file.end_date || null, courseName: _currentCourseName || null, materialTitle: materialTitle || null });
    btn.innerHTML = '<span>' + ICONS.done + ' ' + escapeHtml(file.display_name || file.file_name) + '</span>';
    await invoke('luna_reveal_file', { path: path });
  } catch(e) { btn.innerHTML = orig; btn.disabled = false; showStatus('\u30c0\u30a6\u30f3\u30ed\u30fc\u30c9\u30a8\u30e9\u30fc: ' + String(e)); }
}
async function openMaterialLink(idnumber, file, btn) {
  var invoke = window.__TAURI__?.core?.invoke; if (!invoke) return;
  var orig = btn.innerHTML; btn.innerHTML = '<span>\u958b\u3044\u3066\u3044\u307e\u3059...</span>'; btn.disabled = true;
  try {
    var url;
    if (file.external_url && /^https?:/i.test(file.external_url)) {
      url = file.external_url;
    } else {
      url = await invoke('luna_resolve_material_link', { idnumber: idnumber, fileName: file.file_name, objectName: file.object_name, resourceId: file.resource_id, fileType: file.file_type || '0', materialId: file.material_id || null, displayName: file.display_name || null, endDate: file.end_date || null });
    }
    await invoke('open_external_url', { url: url });
    btn.innerHTML = orig; btn.disabled = false;
  } catch(e) { btn.innerHTML = orig; btn.disabled = false; showStatus('\u30ea\u30f3\u30af\u3092\u958b\u3051\u307e\u305b\u3093\u3067\u3057\u305f: ' + String(e)); }
}
async function checkAndMarkDownloaded(att, btn) {
  var invoke = window.__TAURI__?.core?.invoke; if (!invoke) return;
  var name = att.name || att.file_name || '';
  if (!name) return;
  try {
    var rec = await invoke('check_file_downloaded', { filename: name, courseName: _currentCourseName || null });
    if (rec && rec.file_exists) {
      markAsDownloaded(btn, name, rec.path, att);
    }
  } catch(e) { /* ignore */ }
}
async function checkAndMarkDownloadedBatch(items) {
  var invoke = window.__TAURI__?.core?.invoke; if (!invoke || !items || !items.length) return;
  var names = [];
  var seen = {};
  items.forEach(function(item) {
    var name = item && item.att ? (item.att.name || item.att.file_name || '') : '';
    if (!name || seen[name]) return;
    seen[name] = true;
    names.push(name);
  });
  if (!names.length) return;
  try {
    var found = await invoke('check_files_downloaded', { filenames: names, courseName: _currentCourseName || null });
    if (!found) return;
    items.forEach(function(item) {
      var name = item && item.att ? (item.att.name || item.att.file_name || '') : '';
      var rec = found[name] || found[String(name).toLowerCase()];
      if (rec && rec.file_exists) markAsDownloaded(item.btn, name, rec.path, item.att);
    });
  } catch(e) {
    items.forEach(function(item) {
      checkAndMarkDownloaded(item.att, item.btn);
    });
  }
}
async function downloadAttachment(att, btn) {
  var invoke = window.__TAURI__?.core?.invoke; if (!invoke) return;
  var name = att.name || att.file_name || '';
  // If already marked as downloaded, open file directly
  if (btn && btn._downloadedPath) {
    try {
      await invoke('open_downloaded_file', { path: btn._downloadedPath });
    } catch(e) {
      showStatus('\u30d5\u30a1\u30a4\u30eb\u3092\u958b\u3051\u307e\u305b\u3093: ' + String(e));
    }
    return;
  }
  await forceDownloadAttachment(att, btn);
}
async function forceDownloadAttachment(att, btn) {
  var invoke = window.__TAURI__?.core?.invoke; if (!invoke) return;
  var name = att.name || att.file_name || '';
  var orig = btn ? btn.innerHTML : null;
  if (btn) { btn.innerHTML = '<span>' + ICONS.clip + ' \u30c0\u30a6\u30f3\u30ed\u30fc\u30c9\u4e2d...</span>'; btn.disabled = true; }
  try {
    var result = await invoke('luna_download_file', {
      url: att.url || '',
      filename: name,
      pagePath: _currentPagePath || null,
      objectName: att.object_name || null,
      downloadAction: att.download_action || null,
      downloadParams: att.download_params || null,
      courseName: _currentCourseName || null,
      detailTitle: _currentDetailTitle || null
    });
    if (result.startsWith('http')) {
      await invoke('open_external_url', { url: result });
      if (btn) { btn.innerHTML = orig; btn.disabled = false; }
    } else {
      markAsDownloaded(btn, name, result, att);
      await invoke('luna_reveal_file', { path: result });
    }
  } catch(e) {
    if (btn) { btn.innerHTML = orig; btn.disabled = false; }
    showStatus('\u30c0\u30a6\u30f3\u30ed\u30fc\u30c9\u30a8\u30e9\u30fc: ' + String(e));
  }
}
function markAsDownloaded(btn, name, path, att) {
  if (!btn) return;
  btn.classList.add('downloaded');
  btn._downloadedPath = path;
  btn.disabled = false;
  btn.innerHTML = '<span>' + ICONS.done + ' ' + escapeHtml(name) + '</span><span class="att-redownload" title="\u518d\u30c0\u30a6\u30f3\u30ed\u30fc\u30c9" style="flex:none">' + ICONS.download + '</span>';
  btn.querySelector('.att-redownload').addEventListener('click', function(e) {
    e.stopPropagation();
    btn.classList.remove('downloaded');
    delete btn._downloadedPath;
    btn.innerHTML = '<span>' + ICONS.clip + ' ' + escapeHtml(name) + '</span><span style="flex:none;opacity:0.5">' + ICONS.download + '</span>';
    forceDownloadAttachment(att, btn);
  });
}
async function openExternalLink(url, name) {
  var invoke = window.__TAURI__?.core?.invoke; if (!invoke) return;
  try {
    var fullUrl = url.startsWith('http') ? url : 'https://luna.kwansei.ac.jp' + url;
    await invoke('open_external_url', { url: fullUrl });
  } catch(e) { showStatus('\u30ea\u30f3\u30af\u3092\u958b\u3051\u307e\u305b\u3093\u3067\u3057\u305f: ' + String(e)); }
}
function showStatus(msg, color) {
  var st = document.querySelector('.status') || Object.assign(document.createElement('div'), { className: 'status' });
  st.textContent = msg; st.style.color = color || 'var(--red)';
  document.getElementById('content').appendChild(st);
}

function normalizeCourseName(text) {
  return String(text || '')
    .toLowerCase()
    .replace(/\s+/g, '')
    .replace(/[　]/g, '')
    .trim();
}

function simplifyCourseName(text) {
  return String(text || '')
    .replace(/^.+\s\d{7,8}\s+/, '')
    .replace(/[\[［]\d+[\]］]/g, '')
    .replace(/[（(][^)）]*(?:学期|限|クラス|組|セメスター|Quarter|Semester)[^)）]*[)）]\s*$/i, '')
    .trim();
}

function formatLiveNoteDate(rec) {
  var filename = String(rec && rec.filename || '');
  var m = filename.match(/^(\d{4})(\d{2})(\d{2})_/);
  if (m) return m[1] + '/' + m[2] + '/' + m[3];
  var ts = Number(rec && rec.downloaded_at || 0);
  if (!isNaN(ts) && ts > 0) {
    var d = new Date(ts);
    if (!isNaN(d.getTime())) {
      return d.getFullYear() + '/' + String(d.getMonth() + 1).padStart(2, '0') + '/' + String(d.getDate()).padStart(2, '0');
    }
  }
  return '日付不明';
}

async function loadCourseLiveNotes(courseName) {
  var sec = document.getElementById('live-notes-sec');
  var body = document.getElementById('live-notes-body');
  if (!sec || !body || !courseName) return;
  var invoke = window.__TAURI__?.core?.invoke;
  if (!invoke) return;
  try {
    var records = await invoke('list_downloads');
    var target = normalizeCourseName(courseName);
    var targetSimple = normalizeCourseName(simplifyCourseName(courseName));
    var notes = (records || []).filter(function(rec) {
      var rawName = String(rec && rec.course_name || '');
      var name = normalizeCourseName(rawName);
      var nameSimple = normalizeCourseName(simplifyCourseName(rawName));
      var filename = String(rec && rec.filename || '');
      var hay = normalizeCourseName([rawName, filename, rec && rec.path || ''].join(' '));
      if (!(rec && rec.file_exists) || filename.indexOf('_live.md') < 0) return false;
      if (name && (name === target || name === targetSimple || target.indexOf(name) >= 0 || name.indexOf(target) >= 0)) return true;
      if (nameSimple && (nameSimple === target || nameSimple === targetSimple || targetSimple.indexOf(nameSimple) >= 0 || nameSimple.indexOf(targetSimple) >= 0)) return true;
      return (!!target && hay.indexOf(target) >= 0) || (!!targetSimple && hay.indexOf(targetSimple) >= 0);
    });
    if (!notes.length) return;
    var seen = {};
    notes = notes.filter(function(note) {
      var key = String(note && note.path || '');
      if (!key || seen[key]) return false;
      seen[key] = true;
      return true;
    });
    notes.sort(function(a, b) { return Number(b.downloaded_at || 0) - Number(a.downloaded_at || 0); });
    var h = '<div class="sec-grid live-notes-grid">';
    for (var i = 0; i < notes.length; i++) {
      var note = notes[i];
      h += '<button class="item-card live-note-card" data-live-note-path="' + escapeHtml(note.path || '') + '">';
      h += '<span class="live-note-main"><span class="live-note-sub">' + escapeHtml(formatLiveNoteDate(note)) + '</span></span>';
      h += '<span style="flex:none;opacity:0.5">' + ICONS.external + '</span></button>';
    }
    h += '</div>';
    body.innerHTML = h;
    sec.style.display = '';
    body.querySelectorAll('[data-live-note-path]').forEach(function(btn) {
      btn.addEventListener('click', function() {
        var path = btn.dataset.liveNotePath;
        if (path) invoke('open_downloaded_file', { path: path });
      });
    });
  } catch (e) {
    console.warn('live notes load failed:', e);
  }
}
