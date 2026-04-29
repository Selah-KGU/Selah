// Material Info
function renderMaterialInfo(params) {
  var data = { title: params.get('title') || '', course_name: '', sections: [], attachments: [], meta: [] };
  var period = params.get('period');
  if (period) data.meta.push(['\u516c\u958b\u671f\u9593', period]);
  var desc = params.get('status');
  if (desc) data.sections.push({ heading: '', body: desc });
  renderDetail(data);
  var filesJson = params.get('infoId'), idnumber = params.get('idnumber') || '', files = [];
  if (filesJson) try { files = JSON.parse(filesJson); } catch(e) {}
  if (!files.length) return;
  var c = document.getElementById('content');
  // Split into downloadable files and external links
  var dlFiles = [], linkFiles = [];
  for (var i = 0; i < files.length; i++) {
    var f = files[i]; f._idx = i;
    var lt = f.link_type || (f.file_type === '0' ? 'file' : 'web');
    f._lt = lt;
    if (lt === 'file') dlFiles.push(f); else linkFiles.push(f);
  }
  var fh = '';
  if (dlFiles.length) {
    fh += '<div class="attachments" style="padding:0 24px 24px"><h4>\u6dfb\u4ed8\u30d5\u30a1\u30a4\u30eb (' + dlFiles.length + ')</h4>';
    for (var i = 0; i < dlFiles.length; i++) {
      var f = dlFiles[i];
      fh += '<button class="attachment material-dl" data-idx="' + f._idx + '"><span>' + ICONS.clip + ' ' + escapeHtml(f.display_name || f.file_name) + '</span><span style="flex:none;opacity:0.5">' + ICONS.download + '</span></button>';
    }
    fh += '</div>';
  }
  if (linkFiles.length) {
    fh += '<div class="attachments" style="padding:0 24px 24px"><h4>\u30ea\u30f3\u30af (' + linkFiles.length + ')</h4>';
    for (var i = 0; i < linkFiles.length; i++) {
      var f = linkFiles[i], lt = f._lt, badge = '';
      if (lt === 'zoom') badge = '<span class="link-badge zoom">Zoom</span>';
      else if (lt === 'panopto') badge = '<span class="link-badge panopto">Panopto</span>';
      else if (lt === 'video') badge = '<span class="link-badge video">\u52d5\u753b</span>';
      else if (lt === 'cloud') badge = '<span class="link-badge cloud">Cloud</span>';
      else if (lt === 'google') badge = '<span class="link-badge google">Google</span>';
      else if (lt === 'teams') badge = '<span class="link-badge teams">Teams</span>';
      fh += '<button class="attachment link-item material-link" data-idx="' + f._idx + '"><span>' + ICONS.external + ' ' + badge + escapeHtml(f.display_name || f.file_name) + '</span><span style="flex:none;opacity:0.5">' + ICONS.external + '</span></button>';
    }
    fh += '</div>';
  }
  if (fh) c.insertAdjacentHTML('beforeend', fh);
  c.querySelectorAll('.material-dl').forEach(function(b) {
    b.addEventListener('click', (function(file) { return function() { downloadMaterial(idnumber, file, this, data.title); }; })(files[b.dataset.idx]));
  });
  c.querySelectorAll('.material-link').forEach(function(b) {
    b.addEventListener('click', (function(file) { return function() { openMaterialLink(idnumber, file, this); }; })(files[b.dataset.idx]));
  });
}
