var LUNA_DRAFT_PREFIX = 'selah-luna-draft:v1:';
function lunaDraftKey(parts) {
  var out = [];
  for (var i = 0; i < parts.length; i++) {
    var part = parts[i];
    if (part === null || part === undefined || part === '') continue;
    out.push(encodeURIComponent(String(part)));
  }
  return LUNA_DRAFT_PREFIX + out.join('|');
}
function readDraftValue(key) {
  if (!key) return '';
  try { return localStorage.getItem(key) || ''; } catch(e) { return ''; }
}
function writeDraftValue(key, value) {
  if (!key) return;
  try {
    var text = String(value == null ? '' : value);
    if (text === '') localStorage.removeItem(key);
    else localStorage.setItem(key, text);
  } catch(e) {}
}
function clearDraftValue(key) {
  if (!key) return;
  try { localStorage.removeItem(key); } catch(e) {}
}
function clearDraftValues(keys) {
  if (!keys || !keys.length) return;
  for (var i = 0; i < keys.length; i++) clearDraftValue(keys[i]);
}
function bindDraftField(field, key) {
  if (!field || !key) return key;
  var saved = readDraftValue(key);
  if (!field.value && saved) field.value = saved;
  field.addEventListener('input', function() { writeDraftValue(key, field.value); });
  field.addEventListener('change', function() { writeDraftValue(key, field.value); });
  return key;
}
function attendanceCommentDraftKey(att) {
  var params = new URLSearchParams(window.location.search);
  return lunaDraftKey([
    'attendance-comment',
    att && (att.idnumber || params.get('idnumber')) || '',
    att && att.attendance_id || ''
  ]);
}
