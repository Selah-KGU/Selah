// KGC / Syllabus Detail (key-value fields)
function renderSyllabusLoading(courseName) {
  var c = document.getElementById('content');
  var h = '<div class="detail-wrap">';
  h += '<div class="course-label">授業・時間割照会（詳細）</div>';
  if (courseName) h += '<div class="page-title">' + escapeHtml(courseName) + '</div>';
  h += '<div class="meta-table">';
  for (var i = 0; i < 6; i++) {
    var keyW = 60 + ((i * 13) % 40);
    var valW = 50 + ((i * 17) % 45);
    h += '<div class="meta-row"><span class="meta-key"><span class="skel-bar" style="width:' + keyW + '%"></span></span>';
    h += '<span class="meta-value"><span class="skel-bar" style="width:' + valW + '%"></span></span></div>';
  }
  h += '</div></div>';
  c.innerHTML = h;
}

function parseStandardYearRange(value) {
  // Normalize full-width digits — KGC writes "１年" not "1年".
  var v = (value || '').trim().replace(/[０-９]/g, function(d) {
    return String.fromCharCode(d.charCodeAt(0) - 0xFEE0);
  });
  if (!v) return null;
  if (v.indexOf('全学年') >= 0 || v.indexOf('不問') >= 0) return [1, 99];
  var nums = (v.match(/\d+/g) || []).map(function(n) { return parseInt(n, 10); }).filter(function(n) { return n >= 1 && n <= 10; });
  if (nums.length === 0) return null;
  var min = Math.min.apply(null, nums);
  var max = v.indexOf('以上') >= 0 ? 99 : Math.max.apply(null, nums);
  return [min, max];
}

function renderKgcDetail(data, courseName) {
  var c = document.getElementById('content');
  if (!data) { c.innerHTML = '<div class="error">データがありません</div>'; return; }
  if (data.error) { c.innerHTML = '<div class="error">' + escapeHtml(data.error) + '</div>'; return; }
  var fields = (data.fields || []).slice();
  if (fields.length === 0) { c.innerHTML = '<div class="error">詳細情報を取得できませんでした。</div>'; return; }

  // Promote "履修基準年度 / Standard Year for Registration" to the top so
  // students see whether they can register before scrolling through everything.
  var stdYearIdx = -1;
  for (var k = 0; k < fields.length; k++) {
    var lbl = fields[k][0] || '';
    var lblLower = lbl.toLowerCase();
    if (lbl.indexOf('履修基準年度') >= 0 || lbl.indexOf('履修基準') >= 0
        || lblLower.indexOf('standard year for registration') >= 0
        || lblLower.indexOf('standard year') >= 0) {
      stdYearIdx = k;
      break;
    }
  }
  if (stdYearIdx > 0) {
    var promoted = fields.splice(stdYearIdx, 1)[0];
    fields.unshift(promoted);
  }

  var h = '<div class="detail-wrap">';
  h += '<div class="course-label">授業・時間割照会（詳細）</div>';
  if (courseName) h += '<div class="page-title">' + escapeHtml(courseName) + '</div>';
  h += '<div class="meta-table">';
  for (var i = 0; i < fields.length; i++) {
    var label = fields[i][0] || '', value = fields[i][1] || '—';
    var isStdYearRow = (stdYearIdx >= 0 && i === 0);
    var rowClass = isStdYearRow ? 'meta-row std-year-row' : 'meta-row';
    h += '<div class="' + rowClass + '"><span class="meta-key">' + escapeHtml(label) + '</span>';
    h += '<span class="meta-value">' + linkifyText(value);
    if (isStdYearRow) {
      var range = parseStandardYearRange(value);
      if (range) {
        h += ' <span class="year-range-badge">' + range[0] + '年生以上</span>';
      }
    }
    h += '</span></div>';
  }
  h += '</div></div>';
  c.innerHTML = h;
  hydrateInternalLinkLabels(c);
}
