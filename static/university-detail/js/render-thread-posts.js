function discussionAttachmentControlHtml(controlId) {
  return '<div class="discussion-upload" data-discussion-upload="' + escapeHtml(controlId) + '">'
    + '<input type="file" class="discussion-file-input" multiple style="display:none">'
    + '<div class="discussion-upload-row">'
    + '<button type="button" class="btn secondary discussion-file-pick">' + ICONS.clip + ' 添付</button>'
    + '<span class="discussion-upload-hint">最大10件 / 100MB</span>'
    + '</div>'
    + '<div class="discussion-file-list"></div>'
    + '<div class="discussion-file-error"></div>'
    + '</div>';
}

function formatDiscussionFileSize(bytes) {
  if (bytes < 1024) return bytes + 'B';
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + 'KB';
  return (bytes / (1024 * 1024)).toFixed(1) + 'MB';
}

function validateDiscussionFile(file) {
  if (!file) return 'ファイルを選択してください。';
  if (file.size <= 0) return 'ファイルサイズが0バイトです。';
  if (file.size > 100 * 1024 * 1024) return '「' + file.name + '」は最大サイズ（100MB）を超えています。';
  if ((file.name || '').length > 60) return 'ファイル名は60文字以下にしてください。';
  if (/[\*\|\~:;"%\?</>\\]/.test(file.name || '')) return 'ファイル名に使用できない文字が含まれています。';
  return '';
}

function wireDiscussionAttachmentControl(root, controlId) {
  var wrap = root && root.querySelector('[data-discussion-upload="' + controlId + '"]');
  if (!wrap) return null;
  var input = wrap.querySelector('.discussion-file-input');
  var pick = wrap.querySelector('.discussion-file-pick');
  var list = wrap.querySelector('.discussion-file-list');
  var error = wrap.querySelector('.discussion-file-error');
  var files = [];

  function setError(message) {
    if (!error) return;
    error.textContent = message || '';
  }

  function render() {
    if (!list) return;
    if (!files.length) {
      list.innerHTML = '';
      return;
    }
    var h = '';
    for (var i = 0; i < files.length; i++) {
      h += '<div class="discussion-file-chip" data-file-idx="' + i + '">'
        + '<span>' + ICONS.clip + ' ' + escapeHtml(files[i].name) + '</span>'
        + '<small>' + formatDiscussionFileSize(files[i].size) + '</small>'
        + '<button type="button" class="discussion-file-remove" title="削除">&times;</button>'
        + '</div>';
    }
    list.innerHTML = h;
    list.querySelectorAll('.discussion-file-remove').forEach(function(btn) {
      btn.addEventListener('click', function() {
        var chip = btn.closest('.discussion-file-chip');
        var idx = parseInt(chip.dataset.fileIdx);
        files.splice(idx, 1);
        setError('');
        render();
      });
    });
  }

  function addFiles(fileList) {
    setError('');
    for (var i = 0; i < fileList.length; i++) {
      if (files.length >= 10) {
        setError('添付ファイルは10個以下にしてください。');
        break;
      }
      var file = fileList[i];
      var message = validateDiscussionFile(file);
      if (message) {
        setError(message);
        continue;
      }
      files.push(file);
    }
    if (input) input.value = '';
    render();
  }

  if (pick && input) pick.addEventListener('click', function() { input.click(); });
  if (input) input.addEventListener('change', function() { addFiles(input.files || []); });

  return {
    getFiles: function() { return files.slice(); },
    clear: function() {
      files = [];
      if (input) input.value = '';
      setError('');
      render();
    },
    setDisabled: function(disabled) {
      if (pick) pick.disabled = !!disabled;
      if (input) input.disabled = !!disabled;
      wrap.querySelectorAll('.discussion-file-remove').forEach(function(btn) { btn.disabled = !!disabled; });
    }
  };
}

function arrayBufferToBase64(buf) {
  var bytes = new Uint8Array(buf);
  var binary = '';
  for (var i = 0; i < bytes.length; i += 8192) {
    var chunk = bytes.subarray(i, i + 8192);
    for (var j = 0; j < chunk.length; j++) binary += String.fromCharCode(chunk[j]);
  }
  return btoa(binary);
}

async function readDiscussionAttachmentPayload(picker) {
  if (!picker) return [];
  var files = picker.getFiles();
  var payload = [];
  for (var i = 0; i < files.length; i++) {
    payload.push({
      fileName: files[i].name,
      fileBase64: arrayBufferToBase64(await files[i].arrayBuffer())
    });
  }
  return payload;
}

function renderThreadPosts(container, threadData, threadUrl) {
  if (!container) return;
  _currentCourseName = (threadData && threadData.course_name) || _currentCourseName || null;
  var posts = (threadData && threadData.posts) || [];
  if (!posts.length) {
    container.innerHTML = '<div style="text-align:center;padding:12px;color:var(--text-tertiary);font-size:12px">\u307e\u3060\u6295\u7a3f\u304c\u3042\u308a\u307e\u305b\u3093</div>';
    return;
  }
  var h = '<div class="post-list">';
  h += '<div class="post-list-header">\u6295\u7a3f (' + posts.length + ')</div>';
  for (var i = 0; i < posts.length; i++) {
    var p = posts[i];
    var flags = (p.status || '').split(',');
    var isTeacher = flags.indexOf('teacher') >= 0;
    var isSelf = flags.indexOf('self') >= 0;
    h += '<div class="post-bubble" data-post-idx="' + i + '">';
    h += '<div class="post-header">';
    if (isTeacher) h += '<span class="post-badge teacher">\u6559\u54e1</span>';
    if (isSelf) h += '<span class="post-badge self">\u81ea\u5206</span>';
    h += '<span class="post-author">' + escapeHtml(p.author || '') + '</span>';
    h += '<span class="post-date">' + escapeHtml(p.date || '') + '</span>';
    if (threadUrl && p.thread_id) h += '<button class="post-reply-btn" data-post-id="' + escapeHtml(p.thread_id) + '">\u8fd4\u4fe1</button>';
    h += '</div>';
    h += '<div class="post-body">' + renderRichText(p.content || '') + '</div>';
    if (p.attachments && p.attachments.length) {
      h += '<div class="attachments post-attachments"><h4>\u6dfb\u4ed8\u30d5\u30a1\u30a4\u30eb</h4>';
      for (var ai = 0; ai < p.attachments.length; ai++) {
        var a = p.attachments[ai];
        h += '<button class="attachment post-attachment" data-post-idx="' + i + '" data-att-idx="' + ai + '" data-type="file">';
        h += '<span>' + ICONS.clip + ' ' + escapeHtml(a.name || '') + '</span><span style="flex:none;opacity:0.5">' + ICONS.download + '</span></button>';
      }
      h += '</div>';
    }
    h += '</div>';
  }
  h += '</div>';
  container.innerHTML = h;
  var downloadChecks = [];
  container.querySelectorAll('.post-attachment').forEach(function(btn) {
    var pidx = parseInt(btn.dataset.postIdx);
    var aidx = parseInt(btn.dataset.attIdx);
    var att = ((posts[pidx] || {}).attachments || [])[aidx] || {};
    btn.addEventListener('click', function(e) {
      if (e.target && e.target.classList && e.target.classList.contains('att-redownload')) return;
      downloadAttachment(att, btn);
    });
    downloadChecks.push({ att: att, btn: btn });
  });
  checkAndMarkDownloadedBatch(downloadChecks);
  if (threadUrl) {
    container.querySelectorAll('.post-reply-btn').forEach(function(btn) {
      btn.addEventListener('click', function(e) {
        e.stopPropagation();
        var bubble = btn.closest('.post-bubble');
        if (bubble.querySelector('.post-reply-form')) return;
        var postId = btn.dataset.postId || '';
        var replyDraftKey = lunaDraftKey(['discussion-reply', threadUrl, postId || 'root']);
        var form = document.createElement('div');
        form.className = 'post-reply-form';
        form.innerHTML = '<textarea rows="2" placeholder="\u8fd4\u4fe1\u5185\u5bb9\u3092\u5165\u529b..."></textarea>'
          + discussionAttachmentControlHtml('postReplyAttachments')
          + '<div class="post-reply-actions"><button class="post-reply-cancel">\u30ad\u30e3\u30f3\u30bb\u30eb</button><button class="post-reply-send">\u9001\u4fe1</button></div>';
        bubble.appendChild(form);
        var inlineReplyArea = form.querySelector('textarea');
        var attachmentPicker = wireDiscussionAttachmentControl(form, 'postReplyAttachments');
        bindDraftField(inlineReplyArea, replyDraftKey);
        inlineReplyArea.focus();
        form.querySelector('.post-reply-cancel').addEventListener('click', function() { form.remove(); });
        form.querySelector('.post-reply-send').addEventListener('click', async function() {
          var inv = window.__TAURI__?.core?.invoke;
          var txt = inlineReplyArea.value.trim();
          if (!inv || !txt) return;
          var sendBtn = form.querySelector('.post-reply-send');
          sendBtn.disabled = true;
          sendBtn.textContent = '\u9001\u4fe1\u4e2d...';
          if (attachmentPicker) attachmentPicker.setDisabled(true);
          try {
            var attachments = await readDiscussionAttachmentPayload(attachmentPicker);
            await inv('luna_reply_discussion', { url: threadUrl, content: txt, parentPostId: postId || null, attachments: attachments });
            clearDraftValue(replyDraftKey);
            form.innerHTML = '<div style="font-size:12px;color:var(--green,#34c759);padding:4px 0">\u2713 \u8fd4\u4fe1\u3057\u307e\u3057\u305f</div>';
            setTimeout(async function() {
              try {
                var refreshed = await inv('luna_fetch_thread_posts', { url: threadUrl });
                renderThreadPosts(container, refreshed, threadUrl);
              } catch(e2) {}
            }, 1500);
          } catch(e) {
            alert('\u8fd4\u4fe1\u30a8\u30e9\u30fc: ' + String(e));
            sendBtn.textContent = '\u9001\u4fe1';
            sendBtn.disabled = false;
            if (attachmentPicker) attachmentPicker.setDisabled(false);
          }
        });
      });
    });
  }
}
