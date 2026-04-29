function statusBadge(status) {
  if (!status) return '';
  if (status.indexOf('\u672a') >= 0) return '<span class="item-badge pending">' + escapeHtml(status) + '</span>';
  return '<span class="item-badge done">' + escapeHtml(status) + '</span>';
}
function attendanceBadge(status) {
  if (!status) return '<span class="attendance-status">-</span>';
  if (status.indexOf('\u767b\u9332\u671f\u9593\u5916') >= 0 || status.indexOf('\u30ed\u30b0\u30a4\u30f3\u671f\u9593\u5916') >= 0) {
    return '<span class="attendance-status">' + escapeHtml(status) + '</span>';
  }
  if (status.indexOf('\u53d7\u4ed8') >= 0 || status.indexOf('\u767b\u9332\u671f\u9593') >= 0 || status.indexOf('\u30ed\u30b0\u30a4\u30f3\u671f\u9593') >= 0 || status.indexOf('\u767b\u9332\u53ef\u80fd') >= 0) {
    return '<span class="attendance-badge open">' + escapeHtml(status) + '</span>';
  }
  if (status.indexOf('\u672a\u51fa\u5e2d') >= 0) {
    return '<span class="attendance-badge absent">' + escapeHtml(status) + '</span>';
  }
  if (status.indexOf('\u51fa\u5e2d') >= 0 || status.indexOf('\u6e08') >= 0 || status.indexOf('\u5b8c\u4e86') >= 0) {
    return '<span class="attendance-badge present">' + escapeHtml(status) + '</span>';
  }
  return '<span class="attendance-status">' + escapeHtml(status) + '</span>';
}
function attendanceCardHtml(att, idx) {
  var h = '<div class="attendance-card">';
  h += '<div class="attendance-main"><div class="attendance-title">' + escapeHtml(att.title || ('#' + (idx + 1))) + '</div>';
  if (att.date) h += '<div class="attendance-date">' + escapeHtml(att.date) + '</div>';
  h += '</div>';
  h += '<div class="attendance-right">' + attendanceBadge(att.status || '');
  if (att.can_register && att.attendance_id) {
    h += '<button class="attendance-send-btn" data-att-idx="' + idx + '">\u7f72\u540d\u767b\u5f55</button>';
  }
  h += '</div></div>';
  return h;
}
function attendanceDateTime(att) {
  var s = (att && att.date ? String(att.date) : '').trim();
  if (!s) return NaN;
  var m = s.match(/(\d{4})\/(\d{1,2})\/(\d{1,2})/);
  if (!m) return NaN;
  return new Date(parseInt(m[1], 10), parseInt(m[2], 10) - 1, parseInt(m[3], 10)).getTime();
}

// Urgency helpers for task-style cards
function parseDeadlineFromPeriod(period) {
  // period is like "2026/04/01 00:00 ～ 2026/07/04 23:59" — extract end date only
  if (!period) return 0;
  var parts = period.split(/[~\uff5e]/);
  if (parts.length < 2) return 0;
  var end = parts[1].trim();
  if (!end) return 0;
  return new Date(end.replace(/\//g, '-')).getTime() || 0;
}
function taskUrgency(period, status) {
  if (status && status.indexOf('\u672a') < 0) return 'done';
  var deadline = parseDeadlineFromPeriod(period);
  if (!deadline) return 'normal';
  var diff = deadline - Date.now();
  if (diff <= 0) return 'overdue';
  if (diff < 86400000) return 'critical';
  if (diff < 4 * 86400000) return 'soon';
  return 'normal';
}
function taskUrgencyPct(period, status) {
  if (status && status.indexOf('\u672a') < 0) return 1;
  var deadline = parseDeadlineFromPeriod(period);
  if (!deadline) return 0;
  var diff = deadline - Date.now();
  if (diff <= 0) return 1;
  var horizon = 7 * 86400000;
  if (diff >= horizon) return 0;
  return 1 - diff / horizon;
}
function taskRemaining(period, status) {
  if (status && status.indexOf('\u672a') < 0) return status;
  var deadline = parseDeadlineFromPeriod(period);
  if (!deadline) return '';
  var diff = deadline - Date.now();
  if (diff <= 0) {
    var elapsed = -diff;
    if (elapsed < 3600000) return Math.floor(elapsed / 60000) + '\u5206\u8d85\u904e';
    if (elapsed < 86400000) return Math.floor(elapsed / 3600000) + '\u6642\u9593\u8d85\u904e';
    return Math.floor(elapsed / 86400000) + '\u65e5\u8d85\u904e';
  }
  if (diff < 3600000) return '\u6b8b\u308a' + Math.ceil(diff / 60000) + '\u5206';
  if (diff < 86400000) return '\u6b8b\u308a' + Math.floor(diff / 3600000) + '\u6642\u9593';
  return '\u6b8b\u308a' + Math.floor(diff / 86400000) + '\u65e5';
}
function taskStatusClass(status) {
  if (!status) return '';
  if (status.indexOf('\u672a') >= 0) return 'pending';
  return 'done';
}
function urgencyOrder(period, status) {
  var u = taskUrgency(period, status);
  if (u === 'overdue') return 0;
  if (u === 'critical') return 1;
  if (u === 'soon') return 2;
  if (u === 'normal') return 3;
  return 4; // done
}
function shortPeriod(period) {
  // Extract just the end date part: "～ 2026/07/04 23:59" -> "7/4 23:59"
  if (!period) return '';
  var parts = period.split(/[~\uff5e]/);
  var end = (parts[1] || '').trim();
  if (!end) return period;
  var m = end.match(/(\d+)\/(\d+)\/(\d+)\s+(\d+:\d+)/);
  if (m) return parseInt(m[2]) + '/' + parseInt(m[3]) + ' ' + m[4];
  return end;
}
function pubDate(period) {
  // Extract just the start/publish date: "2026/04/01 00:00 ～ ..." -> "4/1"
  if (!period) return '';
  var parts = period.split(/[~\uff5e]/);
  var start = (parts[0] || '').trim();
  if (!start) return '';
  var m = start.match(/(\d+)\/(\d+)\/(\d+)/);
  if (m) return parseInt(m[2]) + '/' + parseInt(m[3]);
  return start;
}
function shortDate(d) {
  // "2026/04/01 09:00" -> "4/1"
  if (!d) return '';
  var m = d.match(/(\d+)\/(\d+)\/(\d+)/);
  if (m) return parseInt(m[2]) + '/' + parseInt(m[3]);
  return d;
}
