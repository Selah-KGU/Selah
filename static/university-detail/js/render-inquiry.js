// Inquiry (お問い合わせ / メッセージ) detail.
//
// Note on attachments: the inquiry form has ONE file slot per reply
// (`uploadFile` / `inputFileId` / `inputFileName`), unlike forum threads which
// allow multiple. The reply UI mirrors that — a single file picker; clicking
// it again replaces the selection.
function renderInquiryDetail(data) {
  var c = document.getElementById('content');
  if (!data) {
    c.innerHTML = '<div class="error">データがありません</div>';
    return;
  }

  var h = '<div class="detail-wrap">';
  if (data.course_name) {
    h += '<div class="course-label">' + escapeHtml(data.course_name) + '</div>';
  }
  if (data.title) {
    h += '<div class="page-title">' + escapeHtml(data.title) + '</div>';
  }

  var posts = data.posts || [];
  if (!posts.length) {
    h += '<div class="empty" style="text-align:center;padding:24px;color:var(--text-tertiary);font-size:13px">まだメッセージがありません</div>';
  } else {
    h += '<div class="post-list">';
    h += '<div class="post-list-header">メッセージ (' + posts.length + ')</div>';
    for (var i = 0; i < posts.length; i++) {
      var p = posts[i];
      var classes = ['post-bubble', 'inquiry-bubble'];
      if (p.is_self) classes.push('post-self');
      if (p.is_teacher) classes.push('post-teacher');
      h += '<div class="' + classes.join(' ') + '" data-post-idx="' + i + '">';
      h += '<div class="post-header">';
      if (p.is_teacher) h += '<span class="post-badge teacher">教員</span>';
      if (p.is_self) h += '<span class="post-badge self">自分</span>';
      h += '<span class="post-author">' + escapeHtml(p.author || '') + '</span>';
      h += '<span class="post-date">' + escapeHtml(p.date || '') + '</span>';
      h += '</div>';

      var body = p.content_html && p.content_html.trim();
      if (body) {
        h += '<div class="post-body rich-text">' + body + '</div>';
      } else if (p.content_text) {
        h += '<div class="post-body">' + linkifyText(p.content_text) + '</div>';
      }

      if (p.attachments && p.attachments.length) {
        h += '<div class="attachments post-attachments"><h4>添付ファイル</h4>';
        for (var ai = 0; ai < p.attachments.length; ai++) {
          var a = p.attachments[ai];
          h += '<button class="attachment post-attachment" data-post-idx="' + i + '" data-att-idx="' + ai + '" data-type="file">';
          h += '<span>' + ICONS.clip + ' ' + escapeHtml(a.name || '') + '</span>';
          h += '<span style="flex:none;opacity:0.5">' + ICONS.download + '</span>';
          h += '</button>';
        }
        h += '</div>';
      }
      h += '</div>';
    }
    h += '</div>';
  }

  // Reply form — Luna's inquiry page accepts a single attachment per reply.
  var canReply = !!(data.post_action && (data.idnumber || data.inquiry_id));
  if (canReply) {
    h += '<div class="reply-section">';
    h += '<h3>返信</h3>';
    h += '<textarea id="inquiryReplyContent" class="text-input" rows="4" placeholder="メッセージを入力..."></textarea>';
    h += '<div class="inquiry-attachment-row" style="display:flex;align-items:center;gap:8px;margin-top:8px">';
    h += '<input type="file" id="inquiryReplyFile" style="display:none">';
    h += '<button type="button" class="btn secondary" id="inquiryReplyPick">' + ICONS.clip + ' 添付</button>';
    h += '<span id="inquiryReplyFileName" style="font-size:12px;color:var(--text-tertiary)"></span>';
    h += '<button type="button" id="inquiryReplyFileClear" style="display:none;border:none;background:none;color:var(--text-tertiary);cursor:pointer">×</button>';
    h += '</div>';
    h += '<div id="inquiryReplyError" class="highlight-txt" style="margin-top:6px;font-size:12px"></div>';
    h += '<div class="reply-actions"><button id="inquiryReplyBtn" class="btn primary">送信</button></div>';
    h += '</div>';
  }

  h += '</div>';
  c.innerHTML = h;

  // Wire attachment downloads.
  c.querySelectorAll('.post-attachment').forEach(function(btn) {
    var pidx = parseInt(btn.dataset.postIdx);
    var aidx = parseInt(btn.dataset.attIdx);
    var att = ((posts[pidx] || {}).attachments || [])[aidx] || {};
    btn.addEventListener('click', function(e) {
      if (e.target && e.target.classList && e.target.classList.contains('att-redownload')) return;
      downloadAttachment(att, btn);
    });
    checkAndMarkDownloaded(att, btn);
  });

  if (canReply) wireInquiryReplyForm(c, data);
}

function inquiryArrayBufferToBase64(buf) {
  var bytes = new Uint8Array(buf);
  var binary = '';
  for (var i = 0; i < bytes.length; i += 8192) {
    var chunk = bytes.subarray(i, i + 8192);
    for (var j = 0; j < chunk.length; j++) binary += String.fromCharCode(chunk[j]);
  }
  return btoa(binary);
}

function wireInquiryReplyForm(root, data) {
  var textarea = root.querySelector('#inquiryReplyContent');
  var pickBtn = root.querySelector('#inquiryReplyPick');
  var input = root.querySelector('#inquiryReplyFile');
  var nameSpan = root.querySelector('#inquiryReplyFileName');
  var clearBtn = root.querySelector('#inquiryReplyFileClear');
  var submitBtn = root.querySelector('#inquiryReplyBtn');
  var errorBox = root.querySelector('#inquiryReplyError');
  var draftKey = lunaDraftKey(['inquiry-reply', data.idnumber || '', data.inquiry_id || '']);
  bindDraftField(textarea, draftKey);

  var pickedFile = null;
  function setError(msg) {
    if (errorBox) errorBox.textContent = msg || '';
  }
  function renderFile() {
    if (!pickedFile) {
      nameSpan.textContent = '';
      clearBtn.style.display = 'none';
      return;
    }
    nameSpan.textContent = pickedFile.name + ' (' + Math.round(pickedFile.size / 1024) + 'KB)';
    clearBtn.style.display = '';
  }

  pickBtn.addEventListener('click', function() { input.click(); });
  input.addEventListener('change', function() {
    setError('');
    var f = (input.files && input.files[0]) || null;
    if (!f) { pickedFile = null; renderFile(); return; }
    if (f.size <= 0) { setError('ファイルサイズが0バイトです。'); return; }
    if (f.size > 100 * 1024 * 1024) { setError('100MBを超えています。'); return; }
    if ((f.name || '').length > 60) { setError('ファイル名は60文字以下にしてください。'); return; }
    if (/[\*\|\~:;"%\?</>\\]/.test(f.name || '')) { setError('ファイル名に使用できない文字が含まれています。'); return; }
    pickedFile = f;
    renderFile();
  });
  clearBtn.addEventListener('click', function() {
    pickedFile = null;
    if (input) input.value = '';
    setError('');
    renderFile();
  });

  submitBtn.addEventListener('click', async function() {
    var inv = window.__TAURI__?.core?.invoke;
    if (!inv) return;
    var content = textarea.value.trim();
    if (!content && !pickedFile) {
      setError('内容または添付ファイルを指定してください。');
      return;
    }
    setError('');
    submitBtn.disabled = true;
    submitBtn.textContent = '送信中...';
    pickBtn.disabled = true;
    try {
      var attachment = null;
      if (pickedFile) {
        var buf = await pickedFile.arrayBuffer();
        attachment = { fileName: pickedFile.name, fileBase64: inquiryArrayBufferToBase64(buf) };
      }
      var result = await inv('luna_reply_inquiry', {
        url: _currentPagePath || '',
        content: content,
        attachment: attachment
      });
      clearDraftValue(draftKey);
      textarea.value = '';
      pickedFile = null;
      if (input) input.value = '';
      renderFile();
      submitBtn.innerHTML = ICONS.done + ' ' + escapeHtml(result || '送信しました');
      setTimeout(async function() {
        try {
          var refreshed = await inv('luna_fetch_inquiry_detail', { path: _currentPagePath || '' });
          renderInquiryDetail(refreshed);
        } catch (e) {
          submitBtn.disabled = false;
          submitBtn.textContent = '送信';
          pickBtn.disabled = false;
        }
      }, 1500);
    } catch (e) {
      setError('送信エラー: ' + String(e));
      submitBtn.disabled = false;
      submitBtn.textContent = '送信';
      pickBtn.disabled = false;
    }
  });
}
