var DETAIL_SCRIPT_BASE = 'university-detail/js/';
var _detailScriptPromises = Object.create(null);

function loadDetailScript(name) {
  if (_detailScriptPromises[name]) return _detailScriptPromises[name];
  _detailScriptPromises[name] = new Promise(function(resolve, reject) {
    var script = document.createElement('script');
    script.src = DETAIL_SCRIPT_BASE + name;
    script.onload = resolve;
    script.onerror = function() { reject(new Error('detail script load failed: ' + name)); };
    document.head.appendChild(script);
  });
  return _detailScriptPromises[name];
}

function detailRendererScriptsForMode(mode) {
  var scripts = ['render-common.js', 'downloads.js', 'render-detail.js'];
  if (mode === 'material') {
    scripts.push('render-material.js');
  } else if (mode === 'report') {
    scripts.push('report-submit.js');
  } else if (mode === 'course') {
    scripts.push('render-external.js', 'render-course.js', 'attendance-modal.js');
  } else if (mode === 'attendance') {
    scripts.push('render-attendance.js', 'attendance-modal.js');
  } else if (mode === 'discussion') {
    scripts.push('render-thread-posts.js', 'render-discussion.js');
  } else if (mode === 'thread') {
    scripts.push('render-thread-posts.js', 'render-thread.js');
  } else if (mode === 'survey') {
    scripts.push('render-survey.js');
  } else if (mode === 'kgc' || mode === 'syllabus') {
    scripts.push('render-external.js');
  } else if (mode === 'kwic') {
    scripts.push('render-kwic.js');
  }
  return scripts;
}

async function ensureDetailRenderer(mode) {
  var scripts = detailRendererScriptsForMode(mode);
  for (var i = 0; i < scripts.length; i++) {
    await loadDetailScript(scripts[i]);
  }
}

function renderLoadError(err) {
  var c = document.getElementById('content');
  if (!c) return;
  c.innerHTML = '<div class="error">' + escapeHtml(String(err || '詳細ビューの読み込みに失敗しました')) + '</div>';
}

// Router
document.addEventListener('DOMContentLoaded', async function() {
  var params = new URLSearchParams(window.location.search);
  var mode = params.get('mode'), path = decodeHtmlEntities(params.get('path') || '');
  // Hide titlebar for modes that don't need it
  if (mode === 'kgc' || mode === 'syllabus' || mode === 'kwic') {
    document.getElementById('titlebar').style.display = 'none';
  }
  try {
    await ensureDetailRenderer(mode);
  } catch (loadErr) {
    renderLoadError(loadErr);
    return;
  }
  if (mode === 'material') { renderMaterialInfo(params); return; }
  var invoke = window.__TAURI__?.core?.invoke;
  if (!invoke) { renderDetail({ error: 'Tauri IPC \u304c\u5229\u7528\u3067\u304d\u307e\u305b\u3093' }); return; }
  try {
    if (mode === 'course') {
      var idn = params.get('idnumber'), kgcPath = params.get('kgcPath') || '';
      if (!idn) { renderDetail({ error: '\u30b3\u30fc\u30b9\u756a\u53f7\u304c\u6307\u5b9a\u3055\u308c\u3066\u3044\u307e\u305b\u3093' }); return; }
      if (params.get('courseName')) _currentCourseName = params.get('courseName');
      renderCourseDetail(await invoke('luna_fetch_course_detail', { idnumber: idn }), idn, kgcPath);
    } else if (mode === 'attendance') {
      var aid = params.get('idnumber');
      if (!aid) { renderDetail({ error: '\u30b3\u30fc\u30b9\u756a\u53f7\u304c\u6307\u5b9a\u3055\u308c\u3066\u3044\u307e\u305b\u3093' }); return; }
      renderAttendanceDetail(await invoke('luna_fetch_course_detail', { idnumber: aid }), aid);
    } else if (mode === 'announcement') {
      var idn2 = params.get('idnumber'), infoId = params.get('infoId');
      if (!idn2 || !infoId) { renderDetail({ error: '\u30d1\u30e9\u30e1\u30fc\u30bf\u304c\u4e0d\u8db3\u3057\u3066\u3044\u307e\u3059' }); return; }
      if (params.get('courseName')) _currentCourseName = params.get('courseName');
      renderDetail(await invokeLunaDetailWithRetry('luna_fetch_announcement_detail', {
        idnumber: idn2,
        infoId: infoId,
        expectedTitle: params.get('title') || ''
      }, 1));
    } else if (mode === 'report') {
      if (!path) { renderDetail({ error: '\u30d1\u30b9\u304c\u6307\u5b9a\u3055\u308c\u3066\u3044\u307e\u305b\u3093' }); return; }
      _currentPagePath = path;
      if (params.get('courseName')) _currentCourseName = params.get('courseName');
      var reportData;
      try {
        reportData = await invokeLunaDetailWithRetry('luna_fetch_detail', {
          path: path,
          expectedTitle: params.get('title') || ''
        }, 1);
      } catch(detailErr) {
        if (isLunaTransientDetailError(detailErr)) {
          reportData = {
            title: params.get('title') || '',
            course_name: params.get('courseName') || '',
            sections: [],
            attachments: [],
            meta: []
          };
        } else {
          throw detailErr;
        }
      }
      if (!reportData.title) reportData.title = params.get('title') || '';
      if (!reportData.course_name && params.get('courseName')) reportData.course_name = params.get('courseName');
      if ((!reportData.sections || !reportData.sections.length)
        && (!reportData.meta || !reportData.meta.length)
        && (!reportData.attachments || !reportData.attachments.length)) {
        var reportFallback = await buildReportFallbackData(
          path,
          params.get('title') || reportData.title || '',
          params.get('courseName') || reportData.course_name || ''
        );
        if (reportFallback) {
          reportData = reportFallback;
        }
      }
      if ((!reportData.sections || !reportData.sections.length)
        && (!reportData.meta || !reportData.meta.length)
        && (!reportData.attachments || !reportData.attachments.length)) {
        reportData.meta = [['状態', '課題の詳細本文は取得できませんでしたが、提出フォームは利用できます。']];
      }
      renderDetail(reportData);
      await mountReportSubmitForm(invoke, params, path);
    } else if (mode === 'discussion') {
      if (!path) { renderDetail({ error: '\u30d1\u30b9\u304c\u6307\u5b9a\u3055\u308c\u3066\u3044\u307e\u305b\u3093' }); return; }
      renderDiscussion(await invoke('luna_fetch_discussion_detail', { url: path }));
    } else if (mode === 'thread') {
      if (!path) { renderDetail({ error: '\u30d1\u30b9\u304c\u6307\u5b9a\u3055\u308c\u3066\u3044\u307e\u305b\u3093' }); return; }
      renderThreadDetail(await invoke('luna_fetch_thread_posts', { url: path }), path);
    } else if (mode === 'survey') {
      if (!path) { renderDetail({ error: '\u30d1\u30b9\u304c\u6307\u5b9a\u3055\u308c\u3066\u3044\u307e\u305b\u3093' }); return; }
      _currentPagePath = path;
      if (params.get('courseName')) _currentCourseName = params.get('courseName');
      renderSurveyDetail(await invoke('luna_fetch_survey_detail', { path: path }));
    } else if (mode === 'kwic') {
      var kwicId = params.get('informationId') || '';
      if (!kwicId) { renderDetail({ error: '\u901a\u77e5ID\u304c\u6307\u5b9a\u3055\u308c\u3066\u3044\u307e\u305b\u3093' }); return; }
      renderKwicDetail(await invoke('kwic_fetch_detail', {
        informationId: kwicId,
        informationType: params.get('informationType') || '',
        personCategoryCd: params.get('personCategoryCd') || '',
        categoryCd: params.get('categoryCd') || '',
      }));
    } else if (mode === 'kgc') {
      var kgcPath = params.get('path') || '';
      var kgcName = params.get('name') || '授業詳細';
      if (!kgcPath) { renderDetail({ error: 'パスが指定されていません' }); return; }
      renderKgcDetail(await invoke('fetch_course_detail', { path: kgcPath }), kgcName);
    } else if (mode === 'syllabus') {
      var wlabel = params.get('wlabel') || '';
      var sylName = params.get('name') || '授業詳細';
      if (!wlabel) { renderDetail({ error: 'ラベルが指定されていません' }); return; }
      // Show a skeleton immediately — the backend creates the window first
      // and fetches KGC data in the background, emitting `syllabus-ready`
      // (or `syllabus-error`) when it's done.
      renderSyllabusLoading(sylName);
      var event = window.__TAURI__?.event;
      if (!event) {
        renderDetail({ error: 'Tauri event API が利用できません' });
        return;
      }
      var settled = false;
      var unlistenReady = await event.listen('syllabus-ready', async function() {
        if (settled) return;
        settled = true;
        try {
          var detail = await invoke('get_syllabus_detail', { label: wlabel });
          renderKgcDetail(detail, sylName);
        } catch (e) {
          renderDetail({ error: '詳細データの取得に失敗: ' + String(e) });
        }
        try { unlistenReady(); } catch {}
        try { unlistenError(); } catch {}
      });
      var unlistenError = await event.listen('syllabus-error', function(ev) {
        if (settled) return;
        settled = true;
        renderDetail({ error: String(ev?.payload || '取得に失敗しました') });
        try { unlistenReady(); } catch {}
        try { unlistenError(); } catch {}
      });
      // Race condition guard: backend might emit before we register the
      // listener. After listening, optimistically poll the store once.
      try {
        var maybeDetail = await invoke('get_syllabus_detail', { label: wlabel });
        if (maybeDetail && !settled) {
          settled = true;
          renderKgcDetail(maybeDetail, sylName);
          try { unlistenReady(); } catch {}
          try { unlistenError(); } catch {}
        }
      } catch (_) { /* not ready yet — wait for the event */ }
    } else {
      if (!path) { renderDetail({ error: '\u30d1\u30b9\u304c\u6307\u5b9a\u3055\u308c\u3066\u3044\u307e\u305b\u3093' }); return; }
      _currentPagePath = path;
      if (params.get('courseName')) _currentCourseName = params.get('courseName');
      renderDetail(await invokeLunaDetailWithRetry('luna_fetch_detail', {
        path: path,
        expectedTitle: params.get('title') || ''
      }, 1));
    }
  } catch(e) { renderDetail({ error: String(e) }); }
});

// Universal link interceptor
document.addEventListener('click', function(e) {
  var a = e.target.closest?.('a[href]'); if (!a) return;
  var href = a.getAttribute('href'); if (!href || href === '#') return;
  e.preventDefault(); e.stopPropagation();
  (async function() {
    var title = a.textContent.trim() || href;
    var target = await resolveUniversityLinkTarget(href, title);
    if (target && await openResolvedUniversityLink(target)) return;
    var fallback = resolveUniversityUrl(href);
    if (!fallback) return;
    var inv = window.__TAURI__?.core?.invoke;
    if (inv) {
      await inv('kwic_open_link', { url: fallback.toString(), title: title || fallback.toString() });
    }
  })().catch(function(err) {
    console.warn('internal link open failed:', err);
  });
}, true);
