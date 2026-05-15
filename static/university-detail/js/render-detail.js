// Generic Detail
function renderDetail(data) {
  _detailAttachments = data.attachments || [];
  if (data.course_name) _currentCourseName = data.course_name;
  if (data.title) _currentDetailTitle = data.title;
  var c = document.getElementById('content');
  if (data.error) { c.innerHTML = '<div class="error">' + escapeHtml(data.error) + '</div>'; return; }
  var h = '<div class="detail-wrap">';
  if (data.course_name) h += '<div class="course-label">' + escapeHtml(data.course_name) + '</div>';
  if (data.title) h += '<div class="page-title">' + escapeHtml(data.title) + '</div>';
  if (data.sections) for (var i = 0; i < data.sections.length; i++) {
    var s = data.sections[i];
    h += '<div class="section">';
    if (s.heading) h += '<div class="section-heading">' + escapeHtml(s.heading) + '</div>';
    h += '<div class="section-body rich-text">' + renderRichText(s.body) + '</div></div>';
  }
  if (data.meta && data.meta.length) {
    h += '<div class="meta-table">';
    for (var j = 0; j < data.meta.length; j++) {
      var m = data.meta[j];
      h += '<div class="meta-row"><span class="meta-key">' + escapeHtml(m[0]) + '</span><span class="meta-value">' + linkifyText(m[1] || '\u2014') + '</span></div>';
    }
    h += '</div>';
  }
  if (data.attachments && data.attachments.length) {
    // Group by type: files first, then external links
    var fileAtts = [], linkAtts = [];
    for (var k = 0; k < data.attachments.length; k++) {
      var a = data.attachments[k];
      if (a.link_type === 'file' || !a.link_type) fileAtts.push(a);
      else linkAtts.push(a);
    }
    if (fileAtts.length) {
      h += '<div class="attachments"><h4>\u6dfb\u4ed8\u30d5\u30a1\u30a4\u30eb</h4>';
      for (var k = 0; k < fileAtts.length; k++) {
        var a = fileAtts[k];
        var ai = data.attachments.indexOf(a);
        h += '<button class="attachment" data-att-idx="' + ai + '" data-type="file">';
        h += '<span>' + ICONS.clip + ' ' + escapeHtml(a.name) + '</span><span style="flex:none;opacity:0.5">' + ICONS.download + '</span></button>';
      }
      h += '</div>';
    }
    if (linkAtts.length) {
      h += '<div class="attachments"><h4>\u30ea\u30f3\u30af</h4>';
      for (var k = 0; k < linkAtts.length; k++) {
        var a = linkAtts[k];
        var ai = data.attachments.indexOf(a);
        var icon = ICONS.external, badge = '';
        var lt = a.link_type || 'web';
        if (lt === 'zoom') { badge = '<span class="link-badge zoom">Zoom</span>'; }
        else if (lt === 'panopto') { badge = '<span class="link-badge panopto">Panopto</span>'; }
        else if (lt === 'video') { badge = '<span class="link-badge video">\u52d5\u753b</span>'; }
        else if (lt === 'cloud') { badge = '<span class="link-badge cloud">Cloud</span>'; }
        else if (lt === 'google') { badge = '<span class="link-badge google">Google</span>'; }
        else if (lt === 'teams') { badge = '<span class="link-badge teams">Teams</span>'; }
        h += '<button class="attachment link-item" data-att-idx="' + ai + '" data-type="' + lt + '">';
        h += '<span>' + icon + ' ' + badge + escapeHtml(a.name) + '</span><span style="flex:none;opacity:0.5">' + ICONS.external + '</span></button>';
      }
      h += '</div>';
    }
  }
  if (!data.sections?.length && !data.meta?.length && !data.attachments?.length)
    h += '<div class="card-empty" style="padding:40px 0">\u8a73\u7d30\u60c5\u5831\u3092\u53d6\u5f97\u3067\u304d\u307e\u305b\u3093\u3067\u3057\u305f</div>';
  h += '</div>';
  c.innerHTML = h;
  afterFirstPaint(function() { hydrateInternalLinkLabels(c); });
  var downloadChecks = [];
  c.querySelectorAll('.attachment').forEach(function(b) {
    var type = b.dataset.type || 'file';
    var att = _detailAttachments[parseInt(b.dataset.attIdx)] || {};
    if (type === 'file') {
      b.addEventListener('click', function(e) {
        if (e.target && e.target.classList && e.target.classList.contains('att-redownload')) return;
        downloadAttachment(att, b);
      });
      downloadChecks.push({ att: att, btn: b });
    } else {
      b.addEventListener('click', function() { openExternalLink(att.url || '', att.name || ''); });
    }
  });
  checkAndMarkDownloadedBatch(downloadChecks);
}
