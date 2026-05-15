function renderCourseDetail(data, idnumber, kgcPath) {
  var c = document.getElementById('content');
  if (!data) { c.innerHTML = '<div class="error">\u30c7\u30fc\u30bf\u304c\u3042\u308a\u307e\u305b\u3093</div>'; return; }
  _currentCourseName = data.course_name || _currentCourseName || null;

  // Titlebar: materials manager focused on this course + LUNA
  document.getElementById('titlebar').innerHTML =
    '<button class="luna-open-btn" data-materials-course="' + escapeHtml(data.course_name || '') + '" title="\u3053\u306e\u30b3\u30fc\u30b9\u306e\u8cc7\u6599\u3092\u7ba1\u7406"><svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>\u8cc7\u6599\u7ba1\u7406</button>'
    + '<button class="luna-open-btn" data-luna-course-url="https://luna.kwansei.ac.jp/lms/course?idnumber=' + encodeURIComponent(idnumber) + '" title="LUNA\u3067\u958b\u304f"><svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>LUNA</button>';

  // Hero Header
  var h = '<div class="hero">';
  h += '<div class="hero-title-row"><div class="hero-title">' + escapeHtml(data.course_name || '') + '</div></div>';
  // Build tooltip for full info
  var infoTip = [];
  if (data.semester) infoTip.push(data.semester);
  if (data.teachers) infoTip.push(data.teachers);
  if (data.ta_info) infoTip.push('TA: ' + data.ta_info);
  if (data.la_info) infoTip.push('LA: ' + data.la_info);
  h += '<div class="hero-info-row" title="' + escapeHtml(infoTip.join(' / ')) + '">';
  if (data.semester) h += '<span class="hero-label">' + escapeHtml(data.semester) + '</span>';
  if (data.teachers) { if (data.semester) h += '<span class="hero-sep"></span>'; h += '<span class="hero-subtitle hero-teachers">' + ICONS.person + ' ' + escapeHtml(data.teachers) + '</span>'; }
  if (data.ta_info) { h += '<span class="hero-sep"></span><span class="hero-ta-tag"><span class="ta-label">TA</span>' + escapeHtml(data.ta_info) + '</span>'; }
  if (data.la_info) { h += '<span class="hero-sep"></span><span class="hero-ta-tag"><span class="ta-label">LA</span>' + escapeHtml(data.la_info) + '</span>'; }
  h += '</div>';
  var hasMeta = (data.online_tools && data.online_tools.length) || data.syllabus_url;
  if (hasMeta) {
    h += '<div class="hero-meta">';
    if (data.online_tools) for (var t = 0; t < data.online_tools.length; t++) {
      var tool = data.online_tools[t];
      var chipClass = 'hero-badge';
      var chipIcon = '';
      if (tool.url.indexOf('zoom') >= 0) {
        chipClass += ' zoom';
        chipIcon = '<svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor"><path d="M4 5a2 2 0 0 0-2 2v7a4 4 0 0 0 4 4h7a2 2 0 0 0 2-2v-2l4.6 3.07A1 1 0 0 0 21 18.2V8.8a1 1 0 0 0-1.4-.87L15 11V9a4 4 0 0 0-4-4H4z"/></svg>';
      } else if (tool.url.indexOf('panopto') >= 0) {
        chipClass += ' panopto';
        chipIcon = '<svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor"><path d="M8 5v14l11-7z"/></svg>';
      }
      h += '<button class="' + chipClass + '" data-lti-path="' + escapeHtml(tool.url) + '">' + chipIcon + escapeHtml(tool.name) + '</button>';
    }
    if (data.syllabus_url) h += '<button class="hero-badge syllabus" data-syllabus-url="' + escapeHtml(data.syllabus_url) + '"><svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/></svg>\u30b7\u30e9\u30d0\u30b9</button>';
    h += '</div>';
  }
  h += '</div>';

  // Sections
  h += '<div class="sections-scroll">';

  // Announcements
  var annCount = data.announcements ? data.announcements.length : 0;
  if (annCount) {
    h += '<div class="sec">';
    h += '<div class="sec-head"><span class="sec-label">\u304a\u77e5\u3089\u305b</span><span class="sec-count">' + annCount + '</span></div>';
    h += '<div class="sec-grid">';
    for (var a = 0; a < data.announcements.length; a++) {
      var ann = data.announcements[a];
      h += '<button class="item-card" data-ann-id="' + escapeHtml(ann.info_id || '') + '" data-ann-title="' + escapeHtml(ann.title || '') + '">';
      h += '<div class="item-row"><span class="item-title">' + escapeHtml(ann.title) + '</span>';
      if (ann.is_new) h += '<span class="item-badge new">NEW</span>';
      h += '</div>';
      if (ann.start_date) h += '<span class="item-sub">' + shortDate(ann.start_date) + '</span>';
      h += '</button>';
    }
    h += '</div></div>';
  }

  // Attendance
  var attCount = data.attendances ? data.attendances.length : 0;
  if (attCount) {
    var keep = {};
    if (attCount <= 3) {
      for (var ai = 0; ai < attCount; ai++) keep[ai] = true;
    } else {
      var dated = [];
      for (var di = 0; di < attCount; di++) {
        var tm = attendanceDateTime(data.attendances[di]);
        if (!isNaN(tm)) dated.push({ idx: di, time: tm });
      }
      if (dated.length >= 3) {
        dated.sort(function(a, b) { return a.time - b.time; });
        var now = new Date();
        var today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
        var right = 0;
        while (right < dated.length && dated[right].time < today) right++;

        var start = right - 1;
        if (start < 0) start = 0;
        if (start > dated.length - 3) start = dated.length - 3;
        keep[dated[start].idx] = true;
        keep[dated[start + 1].idx] = true;
        keep[dated[start + 2].idx] = true;
      } else {
        keep[0] = true;
        keep[1] = true;
        keep[2] = true;
      }
    }
    var hiddenCount = 0;
    for (var hk = 0; hk < attCount; hk++) {
      if (!keep[hk]) hiddenCount++;
    }

    h += '<div class="sec">';
    h += '<div class="sec-head"><span class="sec-label">\u51fa\u5e2d</span><span class="sec-count">' + attCount + '</span>';
    if (hiddenCount > 0 && idnumber) {
      h += '<button class="attendance-open-btn" data-att-open-window-id="' + escapeHtml(idnumber) + '">\u5168\u3066</button>';
    }
    h += '</div>';
    h += '<div class="attendance-list">';
    for (var at = 0; at < data.attendances.length; at++) {
      var att = data.attendances[at];
      if (keep[at]) h += attendanceCardHtml(att, at);
    }
    h += '</div></div>';
  }

  // Materials
  var matCount = data.materials ? data.materials.length : 0;
  h += '<div class="sec" id="materials-sec"' + (matCount ? '' : ' style="display:none"') + '>';
  h += '<div class="sec-head"><span class="sec-label">\u6559\u6750</span>' + (matCount ? '<span class="sec-count">' + matCount + '</span>' : '') + '</div>';
  h += '<div id="textbook-area" style="display:none"></div>';
  if (matCount) {
    h += '<div class="sec-grid mat-grid">';
    for (var m = 0; m < data.materials.length; m++) {
      var mat = data.materials[m];
      var hasFiles = mat.files && mat.files.length;
      h += '<div class="mat-card" data-material-idx="' + m + '">';
      h += '<div class="mat-title">' + escapeHtml(mat.title) + '</div>';
      if (mat.description) h += '<div class="mat-desc rich-text rich-text-compact">' + renderRichText(mat.description, true) + '</div>';
      if (mat.period) h += '<div class="mat-time">' + pubDate(mat.period) + '</div>';
      if (hasFiles) {
        h += '<div class="mat-files">';
        for (var fi = 0; fi < mat.files.length; fi++) {
          var f = mat.files[fi];
          var flt = f.link_type || (f.file_type === '0' ? 'file' : 'web');
          var isLink = flt !== 'file';
          h += '<button class="mat-file-btn' + (isLink ? ' mat-link-btn' : '') + '" data-mat-idx="' + m + '" data-file-idx="' + fi + '">';
          if (isLink) {
            h += '<svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>';
          } else {
            h += '<svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>';
          }
          h += '<span>' + escapeHtml(f.display_name || f.file_name) + '</span></button>';
        }
        h += '</div>';
      }
      h += '</div>';
    }
    h += '</div>';
  }
  h += '</div>';

  h += '<div class="sec" id="live-notes-sec" style="display:none">';
  h += '<div class="sec-head"><span class="sec-label">LIVE \u30ce\u30fc\u30c8</span></div>';
  h += '<div id="live-notes-body"></div>';
  h += '</div>';

  // Reports
  var repCount = data.reports ? data.reports.length : 0;
  var pendingReps = data.reports ? data.reports.filter(function(r) { return r.status && r.status.indexOf('\u672a') >= 0; }).length : 0;
  if (repCount) {
    h += '<div class="sec">';
    h += '<div class="sec-head"><span class="sec-label">\u8ab2\u984c</span><span class="sec-count">' + repCount + '</span>';
    if (pendingReps > 0) h += '<span class="item-badge pending sec-badge">\u672a\u63d0\u51fa ' + pendingReps + '</span>';
    h += '</div>';
    h += '<div class="task-list">';
    var repIdx = data.reports.map(function(_, i) { return i; });
    repIdx.sort(function(a, b) {
      var da = urgencyOrder(data.reports[a].period, data.reports[a].status);
      var db = urgencyOrder(data.reports[b].period, data.reports[b].status);
      if (da !== db) return da - db;
      var ta = parseDeadlineFromPeriod(data.reports[a].period) || Infinity;
      var tb = parseDeadlineFromPeriod(data.reports[b].period) || Infinity;
      return ta - tb;
    });
    for (var ri = 0; ri < repIdx.length; ri++) {
      var r = repIdx[ri];
      var rep = data.reports[r];
      var urg = taskUrgency(rep.period, rep.status);
      var pct = taskUrgencyPct(rep.period, rep.status);
      var remain = taskRemaining(rep.period, rep.status);
      h += '<button class="task-card" data-report-idx="' + r + '">';
      h += '<div class="urgency-bar ' + urg + '"><div class="urgency-fill" style="height:' + Math.max(Math.round(pct * 100), 6) + '%"></div></div>';
      h += '<div class="task-body">';
      h += '<div class="task-name">' + escapeHtml(rep.title) + '</div>';
      h += '<div class="task-sub">';
      if (rep.status) h += '<span class="task-type">' + escapeHtml(rep.status) + '</span>';
      if (rep.status && rep.period) h += '<span class="task-sep"></span>';
      if (rep.period) h += '<span class="task-date">' + shortPeriod(rep.period) + '</span>';
      h += '</div>';
      h += '</div>';
      if (remain) h += '<span class="remaining ' + urg + '">' + remain + '</span>';
      h += '<svg class="task-arrow" width="7" height="12" viewBox="0 0 7 12" fill="none"><path d="M1 1l5 5-5 5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>';
      h += '</button>';
    }
    h += '</div></div>';
  }

  // Exams
  var exCount = data.examinations ? data.examinations.length : 0;
  var pendingEx = data.examinations ? data.examinations.filter(function(e) { return e.status && e.status.indexOf('\u672a') >= 0; }).length : 0;
  if (exCount) {
    h += '<div class="sec">';
    h += '<div class="sec-head"><span class="sec-label">\u30c6\u30b9\u30c8</span><span class="sec-count">' + exCount + '</span>';
    if (pendingEx > 0) h += '<span class="item-badge pending sec-badge">\u672a\u56de\u7b54 ' + pendingEx + '</span>';
    h += '</div>';
    h += '<div class="task-list">';
    var exIdx = data.examinations.map(function(_, i) { return i; });
    exIdx.sort(function(a, b) {
      var da = urgencyOrder(data.examinations[a].period, data.examinations[a].status);
      var db = urgencyOrder(data.examinations[b].period, data.examinations[b].status);
      if (da !== db) return da - db;
      var ta = parseDeadlineFromPeriod(data.examinations[a].period) || Infinity;
      var tb = parseDeadlineFromPeriod(data.examinations[b].period) || Infinity;
      return ta - tb;
    });
    for (var xi = 0; xi < exIdx.length; xi++) {
      var x = exIdx[xi];
      var exam = data.examinations[x];
      var exUrg = taskUrgency(exam.period, exam.status);
      var exPct = taskUrgencyPct(exam.period, exam.status);
      var exRemain = taskRemaining(exam.period, exam.status);
      h += '<button class="task-card" data-exam-url="' + escapeHtml(exam.url || '') + '" data-exam-title="' + escapeHtml(exam.title) + '">';
      h += '<div class="urgency-bar ' + exUrg + '"><div class="urgency-fill" style="height:' + Math.max(Math.round(exPct * 100), 6) + '%"></div></div>';
      h += '<div class="task-body">';
      h += '<div class="task-name">' + escapeHtml(exam.title) + '</div>';
      h += '<div class="task-sub">';
      if (exam.status) h += '<span class="task-type">' + escapeHtml(exam.status) + '</span>';
      if (exam.status && exam.period) h += '<span class="task-sep"></span>';
      if (exam.period) h += '<span class="task-date">' + shortPeriod(exam.period) + '</span>';
      h += '</div>';
      h += '</div>';
      if (exRemain) h += '<span class="remaining ' + exUrg + '">' + exRemain + '</span>';
      h += '<svg class="task-arrow" width="7" height="12" viewBox="0 0 7 12" fill="none"><path d="M1 1l5 5-5 5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>';
      h += '</button>';
    }
    h += '</div></div>';
  }

  // Discussions
  var discCount = data.discussions ? data.discussions.length : 0;
  if (discCount) {
    h += '<div class="sec">';
    h += '<div class="sec-head"><span class="sec-label">\u63b2\u793a\u677f</span><span class="sec-count">' + discCount + '</span></div>';
    h += '<div class="sec-grid">';
    for (var d = 0; d < data.discussions.length; d++) {
      var disc = data.discussions[d];
      h += '<button class="item-card" data-disc-idx="' + d + '">';
      h += '<div class="item-row"><span class="item-title">' + escapeHtml(disc.title) + '</span>' + statusBadge(disc.status) + '</div>';
      if (disc.period) h += '<span class="item-sub">' + shortPeriod(disc.period) + '</span>';
      h += '</button>';
    }
    h += '</div></div>';
  }

  // Surveys
  var survCount = data.surveys ? data.surveys.length : 0;
  if (survCount) {
    var pendingSurv = data.surveys.filter(function(s) { return s.status && s.status.indexOf('\u672a') >= 0; }).length;
    h += '<div class="sec">';
    h += '<div class="sec-head"><span class="sec-label">\u30a2\u30f3\u30b1\u30fc\u30c8</span><span class="sec-count">' + (pendingSurv ? pendingSurv + '/' : '') + survCount + '</span></div>';
    var survIdx = data.surveys.map(function(_, i) { return i; });
    survIdx.sort(function(a, b) {
      var da2 = urgencyOrder(data.surveys[a].period, data.surveys[a].status);
      var db2 = urgencyOrder(data.surveys[b].period, data.surveys[b].status);
      if (da2 !== db2) return da2 - db2;
      var ta2 = parseDeadlineFromPeriod(data.surveys[a].period) || Infinity;
      var tb2 = parseDeadlineFromPeriod(data.surveys[b].period) || Infinity;
      return ta2 - tb2;
    });
    h += '<div class="sec-grid">';
    for (var sv = 0; sv < survIdx.length; sv++) {
      var surv = data.surveys[survIdx[sv]];
      var su = taskUrgency(surv.period, surv.status);
      h += '<button class="item-card task ' + su + '" data-surv-idx="' + survIdx[sv] + '">';
      h += '<div class="item-row"><span class="item-title">' + escapeHtml(surv.title) + '</span>' + statusBadge(surv.status) + '</div>';
      var srem = taskRemaining(surv.period, surv.status);
      if (srem || surv.period) h += '<span class="item-sub">' + escapeHtml(srem || shortPeriod(surv.period)) + '</span>';
      h += '</button>';
    }
    h += '</div></div>';
  }

  // KG Detail
  if (kgcPath) {
    h += '<div class="sec">';
    h += '<button class="kg-toggle" id="kg-toggle"><span class="kg-toggle-label">' + ICONS.kg + ' KG \u6388\u696d\u8a73\u7d30</span>';
    h += '<span class="kg-chevron" id="kg-chevron">' + ICONS.chevron + '</span></button>';
    h += '<div class="kg-body" id="kg-body" style="display:none"><div class="kg-loading"><div class="spinner"></div><span>\u8aad\u307f\u8fbc\u307f\u4e2d...</span></div></div>';
    h += '</div>';
  }

  h += '</div>';
  c.innerHTML = h;

  // Event Wiring
  var invoke = window.__TAURI__?.core?.invoke;
  if (!invoke) return;

  // LTI tools
  c.querySelectorAll('[data-lti-path]').forEach(function(b) {
    b.addEventListener('click', function() { invoke('open_external_url', { url: 'https://luna.kwansei.ac.jp' + b.dataset.ltiPath }); });
  });
  c.querySelectorAll('[data-syllabus-url]').forEach(function(b) {
    b.addEventListener('click', function() { invoke('open_external_url', { url: b.dataset.syllabusUrl }); });
  });
  c.querySelectorAll('[data-luna-course-url]').forEach(function(b) {
    b.addEventListener('click', function() { invoke('open_external_url', { url: b.dataset.lunaCourseUrl }); });
  });
  document.getElementById('titlebar').querySelectorAll('[data-luna-course-url]').forEach(function(b) {
    b.addEventListener('click', function() { invoke('open_external_url', { url: b.dataset.lunaCourseUrl }); });
  });
  document.getElementById('titlebar').querySelectorAll('[data-materials-course]').forEach(function(b) {
    b.addEventListener('click', function() {
      invoke('open_downloads_window', { focusCourse: b.dataset.materialsCourse || null });
    });
  });
  loadCourseLiveNotes(_currentCourseName || data.course_name || '');

  // Announcements
  c.querySelectorAll('[data-ann-id]').forEach(function(b) {
    b.addEventListener('click', function() {
      var id = b.dataset.annId, title = b.dataset.annTitle;
      if (id && idnumber) invoke('university_open_detail_window', { path: '', title: title, mode: 'announcement', idnumber: idnumber, infoId: id, courseName: _currentCourseName || null });
    });
  });

  // Attendance sign-in
  c.querySelectorAll('.attendance-send-btn[data-att-idx]').forEach(function(btn) {
    btn.addEventListener('click', async function() {
      var idx = parseInt(btn.dataset.attIdx, 10);
      var att = data.attendances && data.attendances[idx];
      if (!att || !att.attendance_id) return;
      var targetId = att.idnumber || idnumber;
      if (!targetId) return;

      var result = await openAttendanceModal(att);
      if (!result) { showStatus('出勤登録をキャンセルしました'); return; }

      var orig = btn.textContent;
      btn.disabled = true;
      btn.textContent = '送信中...';
      try {
        var msg = await invoke('luna_submit_attendance', {
          idnumber: targetId,
          attendanceId: att.attendance_id,
          oneTimePass: result.pass,
          comment: result.comment
        });
        clearDraftValue(attendanceCommentDraftKey(att));
        showStatus(msg || '出席を登録しました', 'var(--green)');
        var fresh = await invoke('luna_fetch_course_detail', { idnumber: targetId });
        renderCourseDetail(fresh, targetId, kgcPath);
      } catch (e) {
        btn.disabled = false;
        btn.textContent = orig || '署名登録';
        showStatus('出席登録エラー: ' + String(e));
      }
    });
  });
  c.querySelectorAll('[data-att-open-window-id]').forEach(function(b) {
    b.addEventListener('click', function() {
      invoke('university_open_detail_window', {
        path: '',
        title: '\u51fa\u5e2d',
        mode: 'attendance',
        idnumber: b.dataset.attOpenWindowId,
        courseName: _currentCourseName || data.course_name || null
      });
    });
  });

  // Materials - individual file download buttons
  c.querySelectorAll('.mat-file-btn[data-mat-idx]').forEach(function(btn) {
    btn.addEventListener('click', function(e) {
      e.stopPropagation();
      var mi = parseInt(btn.dataset.matIdx), fi = parseInt(btn.dataset.fileIdx);
      var item = data.materials[mi]; if (!item || !item.files || !item.files[fi]) return;
      var f = item.files[fi];
      var flt = f.link_type || (f.file_type === '0' ? 'file' : 'web');
      if (flt !== 'file') openMaterialLink(idnumber, f, btn);
      else downloadMaterial(idnumber, f, btn, item.title);
    });
  });
  // Material card click fallback (opens detail window)
  c.querySelectorAll('.mat-card[data-material-idx]').forEach(function(b) {
    b.addEventListener('click', function(e) {
      if (e.target.closest('.mat-file-btn')) return;
      var item = data.materials[parseInt(b.dataset.materialIdx)];
      if (!item) return;
      invoke('university_open_detail_window', { path: '', title: item.title, mode: 'material', period: item.period || null, status: item.description || null, idnumber: idnumber || null, infoId: item.files?.length ? JSON.stringify(item.files) : null, courseName: _currentCourseName || data.course_name || null });
    });
  });

  // Reports toggle
  // Reports - click to open detail
  c.querySelectorAll('[data-report-idx]').forEach(function(b) {
    b.addEventListener('click', function() {
      var item = data.reports[parseInt(b.dataset.reportIdx)];
      if (!item || !item.url) return;
      var up = new URLSearchParams(item.url.split('?')[1] || '');
      var reportId = up.get('reportId') || '';
      invoke('university_open_detail_window', { path: item.url, title: item.title, mode: 'report', idnumber: idnumber || null, infoId: reportId || null, courseName: _currentCourseName || null });
    });
  });

  // Exams — open directly in browser webview (not custom detail parser)
  c.querySelectorAll('[data-exam-url]').forEach(function(b) {
    b.addEventListener('click', function() {
      if (b.dataset.examUrl) invoke('open_external_url', { url: 'https://luna.kwansei.ac.jp' + b.dataset.examUrl, title: b.dataset.examTitle || 'テスト' });
    });
  });

  // Discussions - click to open detail
  c.querySelectorAll('[data-disc-idx]').forEach(function(b) {
    b.addEventListener('click', function() {
      var item = data.discussions[parseInt(b.dataset.discIdx)];
      if (item?.url) invoke('university_open_detail_window', { path: item.url, title: item.title, mode: 'discussion', courseName: _currentCourseName || data.course_name || null });
    });
  });

  // Surveys - click to open detail
  c.querySelectorAll('[data-surv-idx]').forEach(function(b) {
    b.addEventListener('click', function() {
      var item = data.surveys[parseInt(b.dataset.survIdx)];
      if (item?.url) invoke('university_open_detail_window', { path: item.url, title: item.title, mode: 'survey', courseName: _currentCourseName || null });
    });
  });

  // KG Detail
  if (kgcPath) {
    var kgT = document.getElementById('kg-toggle'), kgB = document.getElementById('kg-body'), kgC = document.getElementById('kg-chevron');
    var kgLoaded = false;
    kgT.addEventListener('click', async function() {
      var open = kgB.style.display !== 'none';
      kgB.style.display = open ? 'none' : '';
      kgC.style.transform = open ? '' : 'rotate(180deg)';
      if (!open && !kgLoaded) {
        kgLoaded = true;
        try {
          var detail = await invoke('fetch_course_detail', { path: kgcPath });
          if (detail.fields?.length) {
            var fh = '<div class="meta-table" style="margin:8px 0 0">';
            for (var f = 0; f < detail.fields.length; f++) fh += '<div class="meta-row"><span class="meta-key">' + escapeHtml(detail.fields[f][0]) + '</span><span class="meta-value">' + linkifyText(detail.fields[f][1] || '\u2014') + '</span></div>';
            fh += '</div>';
            kgB.innerHTML = fh;
          } else kgB.innerHTML = '<div style="color:var(--text-tertiary);font-size:12px;padding:16px;text-align:center">\u8a73\u7d30\u60c5\u5831\u306f\u3042\u308a\u307e\u305b\u3093</div>';
        } catch(e) { kgB.innerHTML = '<div style="color:var(--red);font-size:12px;padding:16px">' + escapeHtml(String(e)) + '</div>'; }
      }
    });
  }

  // Auto-load textbook info from cached syllabus data
  (async function() {
    try {
      // Extract kgc_code from luna idnumber: e.g. "2026340010010201" → "34001001"
      var kgcCode = idnumber.length >= 12 ? idnumber.substring(4, 12) : idnumber;
      var result = await invoke('get_kgc_syllabus_fields', { kgcCode: kgcCode });
      if (!result) return;
      var textbooks = result.textbooks || [];
      // Fallback: also check fields for textbook keywords (backwards compat)
      if (textbooks.length === 0) {
        var fields = result.fields || [];
        var textbookLabels = ['教科書', '参考文献', '参考書', '教科書Required texts', '参考文献・資料Reference books', '参考文献・資料等'];
        var found = [];
        for (var i = 0; i < fields.length; i++) {
          var lbl = fields[i][0], val = fields[i][1];
          if (!val || !val.trim()) continue;
          for (var j = 0; j < textbookLabels.length; j++) {
            if (lbl.indexOf(textbookLabels[j]) !== -1) { found.push([lbl, val]); break; }
          }
        }
        if (found.length === 0) return;
        var sec = document.getElementById('textbook-area');
        var matSec = document.getElementById('materials-sec');
        var th = '<div class="textbook-block">';
        for (var k = 0; k < found.length; k++) {
          var label = found[k][0].replace(/Required texts|Reference books/gi, '').replace(/・資料等?/g, '').trim();
          th += '<div class="tb-cat">' + ICONS.book + escapeHtml(label || found[k][0]) + '</div>';
          th += '<div class="tb-item">' + linkifyText(found[k][1]) + '</div>';
        }
        th += '</div>';
        sec.innerHTML = th;
        sec.style.display = '';
        if (matSec) matSec.style.display = '';
        return;
      }
      // Render structured textbooks
      var sec = document.getElementById('textbook-area');
      var matSec = document.getElementById('materials-sec');
      var th = '<div class="textbook-block">';
      var lastCat = '';
      for (var i = 0; i < textbooks.length; i++) {
        var tb = textbooks[i];
        // Category label with book icon
        if (tb.category && tb.category !== lastCat) {
          var catLabel = tb.category.replace(/Required texts|Reference books/gi, '').replace(/・資料等?/g, '').trim();
          th += '<div class="tb-cat">' + ICONS.book + escapeHtml(catLabel || tb.category) + '</div>';
          lastCat = tb.category;
        }
        if (tb.title || tb.author) {
          // Structured entry
          th += '<div class="tb-item tb-structured">';
          if (tb.title) th += '<div class="tb-title">' + escapeHtml(tb.title) + '</div>';
          var meta = [];
          if (tb.author) meta.push(escapeHtml(tb.author));
          if (tb.publisher) meta.push(escapeHtml(tb.publisher));
          if (tb.year) meta.push(escapeHtml(tb.year));
          if (meta.length) th += '<div class="tb-meta">' + meta.join(' / ') + '</div>';
          if (tb.isbn) th += '<div class="tb-isbn">ISBN: ' + escapeHtml(tb.isbn) + '</div>';
          th += '</div>';
        } else if (tb.text) {
          // Plain text fallback
          th += '<div class="tb-item">' + linkifyText(tb.text) + '</div>';
        }
      }
      th += '</div>';
      sec.innerHTML = th;
      sec.style.display = '';
      if (matSec) matSec.style.display = '';
    } catch(e) { console.warn('textbook load:', e); }
  })();
}
