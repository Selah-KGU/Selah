async function mountReportSubmitForm(invoke, params, path) {
  // Detect submission type by fetching the actual submission page
  var repIdnumber = params.get('idnumber'), repReportId = params.get('reportId');
  if (repIdnumber && repReportId) {
    var reportType = 'file';
    try { reportType = await invoke('luna_check_report_type', { idnumber: repIdnumber, reportId: repReportId }); } catch(e) {}
    var allowFile = reportType === 'file' || reportType === 'both';
    var allowText = reportType === 'text' || reportType === 'both';

    var submitWrap = document.createElement('div');
    submitWrap.className = 'report-submit-section';
    var innerHtml = '<div class="report-submit-inner">';

    // Text input area
    if (allowText) {
      innerHtml +=
        '<div class="report-text-area" id="reportTextArea">' +
        '<label class="report-text-label" for="reportTextInput">テキスト入力</label>' +
        '<textarea id="reportTextInput" class="text-input" rows="6" placeholder="ここに提出テキストを入力..."></textarea>' +
        '</div>';
    }

    // File drop zone
    if (allowFile) {
      if (allowText) innerHtml += '<div class="report-divider"><span>または</span></div>';
      innerHtml +=
        '<div class="report-file-zone" id="reportDropZone">' +
        '<div class="report-file-icon">' + ICONS.clip + '</div>' +
        '<div class="report-file-label">ファイルを選択またはドラッグ&ドロップ</div>' +
        '<div class="report-file-hint">PDF, Word, Excel, PPT, 画像, ZIP (最大100MB)</div>' +
        '<input type="file" id="reportFileInput" accept=".pdf,.doc,.docx,.ppt,.pptx,.xls,.xlsx,.txt,.zip,.jpg,.jpeg,.png" style="display:none">' +
        '</div>' +
        '<div id="reportFileInfo" style="display:none">' +
        '<div class="report-file-selected">' +
        '<span id="reportFileName"></span>' +
        '<span id="reportFileSize" style="opacity:0.5;margin-left:8px"></span>' +
        '<button id="reportFileRemove" class="report-remove-btn" title="取消">&times;</button>' +
        '</div>' +
        '</div>';
    }

    innerHtml +=
      '<button id="reportSubmitBtn" class="btn primary" style="width:100%;margin-top:12px" disabled>提出する</button>' +
      '<div id="reportProgress" style="display:none;margin-top:8px">' +
      '<div class="report-progress-bar"><div class="report-progress-fill" id="reportProgressFill"></div></div>' +
      '<div id="reportStatusText" style="font-size:12px;color:var(--text-secondary);margin-top:4px"></div>' +
      '</div>' +
      '</div>';
    submitWrap.innerHTML = innerHtml;
    document.getElementById('content').appendChild(submitWrap);

    var submitBtn2 = document.getElementById('reportSubmitBtn');
    var progressDiv = document.getElementById('reportProgress');
    var progressFill = document.getElementById('reportProgressFill');
    var statusText = document.getElementById('reportStatusText');
    var selectedFile = null;
    var textInput = document.getElementById('reportTextInput');
    var fileInput = allowFile ? document.getElementById('reportFileInput') : null;
    var dropZone = allowFile ? document.getElementById('reportDropZone') : null;
    var fileInfo = allowFile ? document.getElementById('reportFileInfo') : null;
    var fileNameEl = allowFile ? document.getElementById('reportFileName') : null;
    var fileSizeEl = allowFile ? document.getElementById('reportFileSize') : null;
    var removeBtn = allowFile ? document.getElementById('reportFileRemove') : null;
    var reportTextDraftKey = allowText ? lunaDraftKey(['report-text', repIdnumber || '', repReportId || path || _currentPagePath || window.location.search]) : '';

    function formatSize(bytes) {
      if (bytes < 1024) return bytes + 'B';
      if (bytes < 1024*1024) return (bytes/1024).toFixed(1) + 'KB';
      return (bytes/(1024*1024)).toFixed(1) + 'MB';
    }

    function updateSubmitState() {
      var hasText = textInput && textInput.value.trim().length > 0;
      var hasFile = !!selectedFile;
      submitBtn2.disabled = !hasText && !hasFile;
    }

    if (textInput) {
      bindDraftField(textInput, reportTextDraftKey);
      textInput.addEventListener('input', updateSubmitState);
      updateSubmitState();
    }

    if (allowFile) {
      function selectFile(file) {
        if (!file) return;
        if (file.size > 100 * 1024 * 1024) { alert('100MBを超えるファイルは提出できません。'); return; }
        if (file.size <= 0) { alert('ファイルサイズが0バイトです。'); return; }
        selectedFile = file;
        fileNameEl.textContent = file.name;
        fileSizeEl.textContent = formatSize(file.size);
        dropZone.style.display = 'none';
        fileInfo.style.display = '';
        updateSubmitState();
      }

      function clearFile() {
        selectedFile = null;
        fileInput.value = '';
        dropZone.style.display = '';
        fileInfo.style.display = 'none';
        progressDiv.style.display = 'none';
        updateSubmitState();
      }

      dropZone.addEventListener('click', function() { fileInput.click(); });
      fileInput.addEventListener('change', function() { if (fileInput.files[0]) selectFile(fileInput.files[0]); });
      removeBtn.addEventListener('click', clearFile);
      dropZone.addEventListener('dragover', function(e) { e.preventDefault(); dropZone.classList.add('dragover'); });
      dropZone.addEventListener('dragleave', function() { dropZone.classList.remove('dragover'); });
      dropZone.addEventListener('drop', function(e) { e.preventDefault(); dropZone.classList.remove('dragover'); if (e.dataTransfer.files[0]) selectFile(e.dataTransfer.files[0]); });
    }

    submitBtn2.addEventListener('click', async function() {
      var hasText = textInput && textInput.value.trim().length > 0;
      var hasFile = !!selectedFile;
      if (!hasText && !hasFile) return;
      submitBtn2.disabled = true;
      submitBtn2.textContent = '提出中...';
      progressDiv.style.display = '';

      try {
        if (hasFile) {
          // File submission
          statusText.textContent = 'ファイルを読み込み中...';
          progressFill.style.width = '10%';
          var buf = await selectedFile.arrayBuffer();
          var bytes = new Uint8Array(buf);
          statusText.textContent = 'エンコード中...';
          progressFill.style.width = '30%';
          var binary = '';
          for (var i = 0; i < bytes.length; i += 8192) {
            var chunk = bytes.subarray(i, i + 8192);
            for (var j = 0; j < chunk.length; j++) binary += String.fromCharCode(chunk[j]);
          }
          statusText.textContent = 'アップロード中...';
          progressFill.style.width = '50%';
          var result = await invoke('luna_submit_report', {
            idnumber: repIdnumber,
            reportId: repReportId,
            fileName: selectedFile.name,
            fileBase64: btoa(binary)
          });
          clearDraftValue(reportTextDraftKey);
          progressFill.style.width = '100%';
          statusText.textContent = result;
          submitBtn2.textContent = result;
          submitBtn2.className = 'btn success';
        } else {
          // Text submission
          statusText.textContent = '提出中...';
          progressFill.style.width = '50%';
          var result = await invoke('luna_submit_report_text', {
            idnumber: repIdnumber,
            reportId: repReportId,
            submissionText: textInput.value.trim()
          });
          clearDraftValue(reportTextDraftKey);
          progressFill.style.width = '100%';
          statusText.textContent = result;
          submitBtn2.textContent = result;
          submitBtn2.className = 'btn success';
          if (textInput) textInput.disabled = true;
        }
      } catch(e) {
        progressFill.style.width = '0%';
        statusText.textContent = '提出エラー: ' + String(e);
        statusText.style.color = 'var(--red, #e53935)';
        submitBtn2.disabled = false;
        submitBtn2.textContent = '再試行';
      }
    });
  }
}
