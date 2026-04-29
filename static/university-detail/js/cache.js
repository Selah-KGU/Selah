var _linkCacheJson = Object.create(null);
var _linkSchedulePromise = null;
var _linkLunaCourseCache = Object.create(null);
var _linkLunaCourseFetches = Object.create(null);

function readCurrentParam(name) {
  return new URLSearchParams(window.location.search).get(name) || '';
}

async function readJsonCache(key) {
  if (Object.prototype.hasOwnProperty.call(_linkCacheJson, key)) {
    return _linkCacheJson[key];
  }
  var invoke = window.__TAURI__?.core?.invoke;
  if (!invoke) return null;
  try {
    var raw = await invoke('get_data_cache', { key: key });
    _linkCacheJson[key] = raw ? JSON.parse(raw) : null;
  } catch (e) {
    _linkCacheJson[key] = null;
  }
  return _linkCacheJson[key];
}

async function readScheduleSnapshot() {
  if (_linkSchedulePromise) return _linkSchedulePromise;
  var invoke = window.__TAURI__?.core?.invoke;
  if (!invoke) return null;
  _linkSchedulePromise = invoke('get_schedule_snapshot').catch(function() { return null; });
  return _linkSchedulePromise;
}

async function getKgcEntryByPath(path) {
  var snap = await readScheduleSnapshot();
  if (!snap || !snap.raw) return null;
  var pools = []
    .concat(snap.raw.kgc_entries_current || [])
    .concat(snap.raw.kgc_entries_next || []);
  for (var i = 0; i < pools.length; i++) {
    if ((pools[i].detail_path || '') === path) return pools[i];
  }
  return null;
}

async function findKgcPathByCourseName(courseName) {
  var name = String(courseName || '').trim();
  if (!name) return '';
  var snap = await readScheduleSnapshot();
  if (!snap || !snap.raw) return '';
  var pools = []
    .concat(snap.raw.kgc_entries_current || [])
    .concat(snap.raw.kgc_entries_next || []);
  for (var i = 0; i < pools.length; i++) {
    if ((pools[i].name || '').trim() === name && pools[i].detail_path) {
      return pools[i].detail_path;
    }
  }
  return '';
}

async function readKwicItemByInfoId(infoId) {
  if (!infoId) return null;
  var home = await readJsonCache('kwic_home');
  var sections = home && Array.isArray(home.sections) ? home.sections : [];
  for (var si = 0; si < sections.length; si++) {
    var items = Array.isArray(sections[si].items) ? sections[si].items : [];
    for (var ii = 0; ii < items.length; ii++) {
      if ((items[ii].id || '') === infoId) return items[ii];
    }
  }
  return null;
}

function resolveLunaIdnumber(url) {
  return url.searchParams.get('idnumber') || readCurrentParam('idnumber') || '';
}

async function readLunaCourseDetail(idnumber) {
  if (!idnumber) return null;
  if (_linkLunaCourseCache[idnumber]) return _linkLunaCourseCache[idnumber];
  if (_linkLunaCourseFetches[idnumber]) return _linkLunaCourseFetches[idnumber];

  _linkLunaCourseFetches[idnumber] = (async function() {
    var cached = await readJsonCache('luna_course:' + idnumber);
    if (cached && cached.course_name) {
      _linkLunaCourseCache[idnumber] = cached;
      return cached;
    }
    var invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) return cached || null;
    try {
      var fresh = await invoke('luna_fetch_course_detail', { idnumber: idnumber });
      if (fresh) {
        _linkLunaCourseCache[idnumber] = fresh;
        return fresh;
      }
    } catch (e) {}
    return cached || null;
  })();

  try {
    var resolved = await _linkLunaCourseFetches[idnumber];
    if (resolved) _linkLunaCourseCache[idnumber] = resolved;
    return resolved;
  } finally {
    delete _linkLunaCourseFetches[idnumber];
  }
}
