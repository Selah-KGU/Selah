// ── Attendance Modal Controller ──
(function() {
  var modal, closeBtn, metaDiv, passInput, commentInput, commentCount, cancelBtn, submitBtn;
  var _resolveModal = null;
  var _commentDraftKey = '';

  function init() {
    modal       = document.getElementById('att-modal');
    closeBtn    = document.getElementById('att-modal-close');
    metaDiv     = document.getElementById('att-modal-meta');
    passInput   = document.getElementById('att-pass-input');
    commentInput= document.getElementById('att-comment-input');
    commentCount= document.getElementById('att-comment-count');
    cancelBtn   = document.getElementById('att-modal-cancel');
    submitBtn   = document.getElementById('att-modal-submit');

    closeBtn.addEventListener('click', dismiss);
    cancelBtn.addEventListener('click', dismiss);
    modal.addEventListener('click', function(e) { if (e.target === modal) dismiss(); });
    commentInput.addEventListener('input', function() {
      commentCount.textContent = commentInput.value.length;
      writeDraftValue(_commentDraftKey, commentInput.value);
    });
    submitBtn.addEventListener('click', function() {
      var pass = passInput.value.trim();
      if (!pass) { passInput.style.borderColor = 'var(--red)'; passInput.focus(); return; }
      if (_resolveModal) _resolveModal({ pass: pass, comment: commentInput.value });
    });
    passInput.addEventListener('input', function() { passInput.style.borderColor = ''; });
    document.addEventListener('keydown', function(e) {
      if (modal && modal.style.display !== 'none' && e.key === 'Escape') dismiss();
    });
  }

  function dismiss() {
    if (modal) modal.style.display = 'none';
    if (_resolveModal) { var r = _resolveModal; _resolveModal = null; r(null); }
  }

  // Returns Promise<{pass, comment}|null>
  window.openAttendanceModal = async function(att) {
    if (!modal) init();
    passInput.value = ''; passInput.style.borderColor = '';
    _commentDraftKey = attendanceCommentDraftKey(att);
    commentInput.value = readDraftValue(_commentDraftKey);
    commentCount.textContent = String(commentInput.value.length);
    metaDiv.style.display = 'none'; metaDiv.innerHTML = '';
    submitBtn.disabled = false;
    modal.style.display = 'flex';
    passInput.focus();

    // Prefetch time-window info in the background
    var invoke = window.__TAURI__?.core?.invoke;
    if (invoke && att && att.idnumber && att.attendance_id) {
      invoke('luna_prefetch_attendance_form', {
        idnumber: att.idnumber,
        attendanceId: att.attendance_id
      }).then(function(info) {
        if (!info || info.already_registered || modal.style.display === 'none') return;
        var rows = '';
        if (info.open_start || info.open_end) {
          rows += '<div class="att-modal-meta-row">'
            + '<span class="att-modal-meta-label">受付時間</span>'
            + '<span class="att-modal-meta-value">' + escapeHtml(info.open_start || '') + ' ～ ' + escapeHtml(info.open_end || '') + '</span>'
            + '</div>';
        }
        if (info.late_start || info.late_end) {
          rows += '<div class="att-modal-meta-row">'
            + '<span class="att-modal-meta-label">遅刻時間</span>'
            + '<span class="att-modal-meta-value">' + escapeHtml(info.late_start || '') + ' ～ ' + escapeHtml(info.late_end || '') + '</span>'
            + '</div>';
        }
        if (info.content) {
          rows += '<div class="att-modal-meta-row">'
            + '<span class="att-modal-meta-label">内容</span>'
            + '<span class="att-modal-meta-value">' + escapeHtml(info.content) + '</span>'
            + '</div>';
        }
        if (rows) { metaDiv.innerHTML = rows; metaDiv.style.display = 'flex'; }
      }).catch(function() {});
    }

    return new Promise(function(resolve) {
      _resolveModal = function(result) { _resolveModal = null; modal.style.display = 'none'; resolve(result); };
    });
  };
})();
