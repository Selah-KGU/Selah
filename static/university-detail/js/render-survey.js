// Course Detail (Bento Layout)
function renderSurveyDetail(data) {
  var c = document.getElementById('content');
  if (!data) { c.innerHTML = '<div class="error">\u30c7\u30fc\u30bf\u304c\u3042\u308a\u307e\u305b\u3093</div>'; return; }
  _currentCourseName = data.course_name || _currentCourseName || null;
  var h = '<div class="detail-wrap">';
  if (data.title) h += '<div class="page-title">' + escapeHtml(data.title) + '</div>';
  if (data.description) h += '<div class="section"><div class="section-body rich-text">' + renderRichText(data.description) + '</div></div>';
  // Info chips
  var chips = [];
  if (data.period) chips.push(['\u671f\u9593', data.period]);
  if (data.anonymity) chips.push(['\u533f\u540d\u6027', data.anonymity]);
  if (data.allow_edit) chips.push(['\u56de\u7b54\u306e\u5909\u66f4', data.allow_edit]);
  if (data.answer_status) chips.push(['\u72b6\u614b', data.answer_status]);
  if (data.respondent) chips.push(['\u56de\u7b54\u8005', data.respondent]);
  if (chips.length) {
    h += '<div class="survey-info-bar">';
    for (var j = 0; j < chips.length; j++) h += '<span class="survey-info-chip">' + escapeHtml(chips[j][0]) + ' <strong>' + escapeHtml(chips[j][1]) + '</strong></span>';
    h += '</div>';
  }
  if (data.attachments && data.attachments.length) {
    h += '<div class="attachments" style="margin:12px 24px 0"><h4>\u6dfb\u4ed8\u30d5\u30a1\u30a4\u30eb</h4>';
    for (var k = 0; k < data.attachments.length; k++) {
      var att = data.attachments[k];
      h += '<button class="attachment survey-att" data-surv-att-idx="' + k + '">';
      h += '<span>' + ICONS.clip + ' ' + escapeHtml(att.file_name) + '</span><span style="flex:none;opacity:0.5">' + ICONS.download + '</span></button>';
    }
    h += '</div>';
  }
  if (data.questions && data.questions.length) {
    h += '<div class="survey-questions">';
    for (var qi = 0; qi < data.questions.length; qi++) {
      var q = data.questions[qi];
      h += '<div class="survey-q-card" data-qi="' + qi + '">';
      h += '<div class="survey-q-header"><span class="survey-q-num">Q' + escapeHtml(q.number) + '</span>';
      if (q.required) h += '<span class="survey-q-req">\u5fc5\u9808</span>';
      h += '</div>';
      h += '<div class="survey-q-body rich-text">' + renderRichText(q.body) + '</div>';
      if (q.options && q.options.length) {
        h += '<div class="survey-q-options">';
        for (var oi = 0; oi < q.options.length; oi++) {
          var opt = q.options[oi];
          var inputType = q.answer_type === 'checkbox' ? 'checkbox' : 'radio';
          if (q.answer_type === 'list') {
            if (oi === 0) h += '<select class="survey-select" name="sq' + qi + '"><option value="">-- \u9078\u629e\u3057\u3066\u304f\u3060\u3055\u3044 --</option>';
            h += '<option value="' + escapeHtml(opt.value) + '">' + escapeHtml(opt.label) + '</option>';
            if (oi === q.options.length - 1) h += '</select>';
          } else {
            h += '<label class="survey-opt-label"><input type="' + inputType + '" name="sq' + qi + '" value="' + escapeHtml(opt.value) + '"> ' + escapeHtml(opt.label) + '</label>';
          }
        }
        h += '</div>';
      }
      if (q.answer_type === 'text' || q.answer_type === 'textarea') {
        h += '<div class="survey-q-options">';
        if (q.answer_type === 'text') {
          h += '<input class="survey-text-input survey-line-input" type="text" name="sq' + qi + '" placeholder="\u56de\u7b54\u3092\u5165\u529b">';
        } else {
          h += '<textarea class="survey-text-input" rows="2" name="sq' + qi + '" placeholder="\u81ea\u7531\u8a18\u8ff0"></textarea>';
        }
        h += '</div>';
      }
      h += '</div>';
    }
    h += '</div>';
    // Submit area
    h += '<div class="survey-submit-area"><div style="text-align:center">';
    h += '<button class="survey-submit-btn" id="surveySubmitBtn">\u56de\u7b54\u3092\u63d0\u51fa</button>';
    h += '<div class="survey-submit-status" id="surveySubmitStatus"></div>';
    h += '</div></div>';
  }
  if (!data.questions?.length && !data.description) h += '<div class="card-empty" style="padding:40px 0">\u8a73\u7d30\u60c5\u5831\u3092\u53d6\u5f97\u3067\u304d\u307e\u305b\u3093\u3067\u3057\u305f</div>';
  h += '</div>';
  c.innerHTML = h;
  // Attachment download
  c.querySelectorAll('.survey-att').forEach(function(b) {
    var sa = data.attachments[parseInt(b.dataset.survAttIdx)];
    if (!sa) return;
    var attObj = { url: sa.url || '', name: sa.file_name, object_name: sa.object_name || '', download_action: sa.download_action || '', download_params: sa.download_params || null };
    b.addEventListener('click', function(e) {
      if (e.target && e.target.classList && e.target.classList.contains('att-redownload')) return;
      downloadAttachment(attObj, b);
    });
    checkAndMarkDownloaded(attObj, b);
  });
  var surveyTextDraftKeys = [];
  if (data.questions && data.questions.length) {
    for (var qi = 0; qi < data.questions.length; qi++) {
      if (data.questions[qi].answer_type !== 'text' && data.questions[qi].answer_type !== 'textarea') continue;
      var textInput = c.querySelector('.survey-q-card[data-qi="' + qi + '"] .survey-text-input');
      var textDraftKey = lunaDraftKey(['survey-text', _currentPagePath || window.location.search, qi]);
      bindDraftField(textInput, textDraftKey);
      surveyTextDraftKeys.push(textDraftKey);
    }
  }
  // Submit handler
  var submitBtn = document.getElementById('surveySubmitBtn');
  var statusEl = document.getElementById('surveySubmitStatus');
  if (submitBtn && data.form_fields && data.form_fields.length) {
    submitBtn.addEventListener('click', async function() {
      var inv = window.__TAURI__?.core?.invoke; if (!inv) return;
      // Collect answers
      var answers = {};
      var qLen = data.questions.length;
      var missing = [];
      for (var i = 0; i < qLen; i++) {
        var q = data.questions[i];
        var els = c.querySelectorAll('[name="sq' + i + '"]');
        var val = '';
        if (q.answer_type === 'list') {
          val = els[0] ? els[0].value : '';
        } else if (q.answer_type === 'text' || q.answer_type === 'textarea') {
          val = els[0] ? els[0].value : '';
        } else if (q.answer_type === 'checkbox') {
          val = [];
          els.forEach(function(el) { if (el.checked) val.push(el.value); });
        } else {
          els.forEach(function(el) { if (el.checked) val = el.value; });
        }
        answers[String(i)] = { name: q.answer_name || '', value: val };
        var hasAnswer = Array.isArray(val) ? val.length > 0 : !!val;
        if (q.required && !hasAnswer) missing.push('Q' + q.number);
      }
      if (missing.length) {
        statusEl.textContent = '\u672a\u56de\u7b54: ' + missing.join(', ');
        statusEl.style.color = 'var(--red)';
        return;
      }
      submitBtn.disabled = true;
      submitBtn.textContent = '\u63d0\u51fa\u4e2d...';
      statusEl.textContent = '';
      statusEl.style.color = '';
      try {
        await inv('luna_submit_survey', { formFields: data.form_fields, answers: answers });
        clearDraftValues(surveyTextDraftKeys);
        submitBtn.textContent = '\u63d0\u51fa\u5b8c\u4e86';
        statusEl.textContent = '\u56de\u7b54\u304c\u63d0\u51fa\u3055\u308c\u307e\u3057\u305f';
        statusEl.style.color = 'var(--green)';
      } catch(e) {
        statusEl.textContent = '\u63d0\u51fa\u30a8\u30e9\u30fc: ' + String(e);
        statusEl.style.color = 'var(--red)';
        submitBtn.disabled = false;
        submitBtn.textContent = '\u518d\u8a66\u884c';
      }
    });
  } else if (submitBtn && (!data.form_fields || !data.form_fields.length)) {
    submitBtn.disabled = true;
    submitBtn.style.opacity = '0.4';
    statusEl.textContent = '\u30d5\u30a9\u30fc\u30e0\u60c5\u5831\u3092\u53d6\u5f97\u3067\u304d\u307e\u305b\u3093\u3067\u3057\u305f';
  }
}
