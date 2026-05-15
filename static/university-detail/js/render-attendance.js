function renderAttendanceDetail(data, idnumber) {
  var c = document.getElementById('content');
  if (!data) { c.innerHTML = '<div class="error">\u30c7\u30fc\u30bf\u304c\u3042\u308a\u307e\u305b\u3093</div>'; return; }
  _currentCourseName = data.course_name || _currentCourseName || null;

  document.getElementById('titlebar').innerHTML = '<button class="luna-open-btn" data-luna-course-url="https://luna.kwansei.ac.jp/lms/course?idnumber=' + encodeURIComponent(idnumber) + '#attendance" title="LUNA\u3067\u958b\u304f"><svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>LUNA</button>';

  var h = '<div class="detail-wrap">';
  if (data.course_name) h += '<div class="course-label">' + escapeHtml(data.course_name) + '</div>';
  h += '<div class="page-title">\u51fa\u5e2d</div>';

  var list = data.attendances || [];
  if (!list.length) {
    h += '<div class="card-empty" style="padding:40px 0">\u51fa\u5e2d\u60c5\u5831\u304c\u3042\u308a\u307e\u305b\u3093</div>';
  } else {
    h += '<div class="attendance-list">';
    for (var i = 0; i < list.length; i++) h += attendanceCardHtml(list[i], i);
    h += '</div>';
  }
  h += '</div>';
  c.innerHTML = h;

  var invoke = window.__TAURI__?.core?.invoke;
  if (!invoke) return;
  document.getElementById('titlebar').querySelectorAll('[data-luna-course-url]').forEach(function(b) {
    b.addEventListener('click', function() { invoke('open_external_url', { url: b.dataset.lunaCourseUrl }); });
  });
  c.querySelectorAll('.attendance-send-btn[data-att-idx]').forEach(function(btn) {
    btn.addEventListener('click', async function() {
      var idx = parseInt(btn.dataset.attIdx, 10);
      var att = list[idx];
      if (!att || !att.attendance_id) return;
      var result = await openAttendanceModal(att);
      if (!result) { showStatus('出勤登録をキャンセルしました'); return; }
      var orig = btn.textContent;
      btn.disabled = true;
      btn.textContent = '送信中...';
      try {
        var targetId = att.idnumber || idnumber;
        var msg = await invoke('luna_submit_attendance', { idnumber: targetId, attendanceId: att.attendance_id, oneTimePass: result.pass, comment: result.comment });
        clearDraftValue(attendanceCommentDraftKey(att));
        showStatus(msg || '出席を登録しました', 'var(--green)');
        var fresh = await invoke('luna_fetch_course_detail', { idnumber: targetId });
        renderAttendanceDetail(fresh, targetId);
      } catch (e) {
        btn.disabled = false;
        btn.textContent = orig || '署名登録';
        showStatus('出席登録エラー: ' + String(e));
      }
    });
  });
}
