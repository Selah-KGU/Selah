// Discussion Thread List
function renderDiscussion(data) {
  var c = document.getElementById('content');
  if (!data) { c.innerHTML = '<div class="error">\u30c7\u30fc\u30bf\u304c\u3042\u308a\u307e\u305b\u3093</div>'; return; }
  _currentCourseName = data.course_name || _currentCourseName || null;
  var params = new URLSearchParams(window.location.search);
  var discussionPath = params.get('path') || '';
  var pp = new URLSearchParams(discussionPath.split('?')[1] || '');
  var idnumber = pp.get('idnumber') || '', forumId = pp.get('forumId') || '';
  var h = '<div class="detail-wrap">';
  if (data.course_name) h += '<div class="course-label">' + escapeHtml(data.course_name) + '</div>';
  if (data.title) h += '<div class="page-title">' + escapeHtml(data.title) + '</div>';
  if (data.description) h += '<div class="section"><div class="section-body rich-text">' + renderRichText(data.description) + '</div></div>';
  if (data.meta && data.meta.length) {
    h += '<div class="meta-table">';
    for (var j = 0; j < data.meta.length; j++) {
      var m = data.meta[j];
      h += '<div class="meta-row"><span class="meta-key">' + escapeHtml(m[0]) + '</span><span class="meta-value">' + linkifyText(m[1] || '\u2014') + '</span></div>';
    }
    h += '</div>';
  }
  if (data.posts && data.posts.length) {
    h += '<div class="thread-list">';
    for (var i = 0; i < data.posts.length; i++) {
      var p = data.posts[i];
      h += '<div class="thread-card" data-thread-id="' + escapeHtml(p.thread_id || '') + '">';
      if (p.title) h += '<div class="thread-title">' + escapeHtml(p.title) + '</div>';
      if (p.content) h += '<div class="thread-body rich-text">' + renderRichText(p.content) + '</div>';
      h += '<div class="thread-meta">';
      if (p.author) h += '<span class="author">' + escapeHtml(p.author) + '</span>';
      if (p.date) h += '<span>' + escapeHtml(p.date) + '</span>';
      if (p.status) h += '<span style="margin-left:auto">' + escapeHtml(p.status) + '</span>';
      h += '</div></div>';
    }
    h += '</div>';
  }
  // New thread form
  h += '<div class="reply-section"><h3>新しいスレッド</h3>';
  h += '<input id="newThreadTitle" class="text-input" type="text" placeholder="タイトル..." style="margin-bottom:8px">';
  h += '<textarea id="newThreadContent" class="text-input" rows="4" placeholder="内容を入力..."></textarea>';
  h += discussionAttachmentControlHtml('newThreadAttachments');
  h += '<div class="reply-actions"><button id="newThreadBtn" class="btn primary">投稿</button></div></div>';

  h += '</div>';
  c.innerHTML = h;
  // Thread card click -> show thread detail inline
  c.querySelectorAll('.thread-card').forEach(function(card, idx) {
    card.addEventListener('click', function() {
      var post = data.posts[idx];
      if (!post) return;
      showThreadInline(c, data, post, idnumber, forumId, discussionPath);
    });
  });
  wireNewThreadForm(c, discussionPath);
}

function showThreadInline(c, discData, post, idnumber, forumId, discussionPath) {
  var h = '<div class="detail-wrap">';
  h += '<button class="back-btn" id="backToList">' + ICONS.chevron + ' \u30b9\u30ec\u30c3\u30c9\u4e00\u89a7</button>';
  if (post.title) h += '<div class="page-title">' + escapeHtml(post.title) + '</div>';
  h += '<div class="thread-meta" style="margin-bottom:12px">';
  if (post.author) h += '<span class="author">' + escapeHtml(post.author) + '</span>';
  if (post.date) h += '<span>' + escapeHtml(post.date) + '</span>';
  if (post.status) h += '<span style="margin-left:auto">' + escapeHtml(post.status) + '</span>';
  h += '</div>';
  if (post.content) h += '<div class="section"><div class="section-body rich-text">' + renderRichText(post.content) + '</div></div>';
  var tid = post.thread_id || '';
  // Posts placeholder
  h += '<div id="threadPostsArea"><div class="loading-text" style="text-align:center;padding:20px;color:var(--text-tertiary);font-size:13px">\u8aad\u307f\u8fbc\u307f\u4e2d...</div></div>';
  if (tid && idnumber && forumId) {
    h += '<div class="reply-section"><h3>\u8fd4\u4fe1</h3>';
    h += '<textarea id="replyContent" class="text-input" rows="4" placeholder="\u8fd4\u4fe1\u5185\u5bb9\u3092\u5165\u529b..."></textarea>';
    h += discussionAttachmentControlHtml('threadReplyAttachments');
    h += '<div class="reply-actions"><button id="replyBtn" class="btn primary">\u9001\u4fe1</button></div></div>';
  }
  h += '</div>';
  c.innerHTML = h;
  document.getElementById('backToList').addEventListener('click', function() {
    renderDiscussion(discData);
  });
  // Fetch thread posts
  if (tid && idnumber && forumId) {
    var tUrl = '/lms/course/forums/thread?idnumber=' + idnumber + '&forumId=' + forumId + '&threadId=' + tid + '&screen=2';
    (async function() {
      var inv = window.__TAURI__?.core?.invoke;
      if (!inv) return;
      try {
        var threadData = await inv('luna_fetch_thread_posts', { url: tUrl });
        renderThreadPosts(document.getElementById('threadPostsArea'), threadData, tUrl);
      } catch(e) {
        var area = document.getElementById('threadPostsArea');
        if (area) area.innerHTML = '<div style="text-align:center;padding:12px;color:var(--text-tertiary);font-size:12px">\u6295\u7a3f\u3092\u53d6\u5f97\u3067\u304d\u307e\u305b\u3093\u3067\u3057\u305f</div>';
      }
    })();
    var replyBtn = document.getElementById('replyBtn'), replyArea = document.getElementById('replyContent');
    var attachmentPicker = wireDiscussionAttachmentControl(c, 'threadReplyAttachments');
    var replyDraftKey = lunaDraftKey(['discussion-reply', tUrl, 'root']);
    bindDraftField(replyArea, replyDraftKey);
    if (replyBtn) {
      replyBtn.addEventListener('click', async function() {
        var inv = window.__TAURI__?.core?.invoke;
        if (!inv || !replyArea.value.trim()) return;
        replyBtn.disabled = true; replyBtn.textContent = '\u9001\u4fe1\u4e2d...';
        if (attachmentPicker) attachmentPicker.setDisabled(true);
        try {
          var attachments = await readDiscussionAttachmentPayload(attachmentPicker);
          var result = await inv('luna_reply_discussion', { url: tUrl, content: replyArea.value.trim(), parentPostId: null, attachments: attachments });
          replyArea.value = '';
          if (attachmentPicker) attachmentPicker.clear();
          clearDraftValue(replyDraftKey);
          replyBtn.innerHTML = ICONS.done + ' ' + escapeHtml(result);
          // Refresh posts after reply
          setTimeout(async function() {
            replyBtn.textContent = '\u9001\u4fe1'; replyBtn.disabled = false;
            if (attachmentPicker) attachmentPicker.setDisabled(false);
            try {
              var threadData = await inv('luna_fetch_thread_posts', { url: tUrl });
              renderThreadPosts(document.getElementById('threadPostsArea'), threadData, tUrl);
            } catch(e2) {}
          }, 1500);
        } catch(e) {
          alert('\u8fd4\u4fe1\u30a8\u30e9\u30fc: ' + String(e));
          replyBtn.textContent = '\u9001\u4fe1'; replyBtn.disabled = false;
          if (attachmentPicker) attachmentPicker.setDisabled(false);
        }
      });
    }
  } else {
    var area = document.getElementById('threadPostsArea');
    if (area) area.innerHTML = '';
  }
}

function wireNewThreadForm(c, discussionPath) {
  var newBtn = document.getElementById('newThreadBtn');
  var newTitle = document.getElementById('newThreadTitle');
  var newContent = document.getElementById('newThreadContent');
  var attachmentPicker = wireDiscussionAttachmentControl(c, 'newThreadAttachments');
  var titleDraftKey = lunaDraftKey(['discussion-new-thread', discussionPath || _currentPagePath || window.location.search, 'title']);
  var contentDraftKey = lunaDraftKey(['discussion-new-thread', discussionPath || _currentPagePath || window.location.search, 'content']);
  bindDraftField(newTitle, titleDraftKey);
  bindDraftField(newContent, contentDraftKey);
  if (newBtn && discussionPath) {
    newBtn.addEventListener('click', async function() {
      var inv = window.__TAURI__?.core?.invoke;
      if (!inv || !newTitle.value.trim() || !newContent.value.trim()) return;
      newBtn.disabled = true; newBtn.textContent = '\u6295\u7a3f\u4e2d...';
      if (attachmentPicker) attachmentPicker.setDisabled(true);
      try {
        var attachments = await readDiscussionAttachmentPayload(attachmentPicker);
        var result = await inv('luna_post_discussion', { url: discussionPath, title: newTitle.value.trim(), content: newContent.value.trim(), attachments: attachments });
        newTitle.value = ''; newContent.value = '';
        if (attachmentPicker) attachmentPicker.clear();
        clearDraftValues([titleDraftKey, contentDraftKey]);
        newBtn.innerHTML = ICONS.done + ' ' + escapeHtml(result);
        setTimeout(function() { newBtn.textContent = '\u6295\u7a3f'; newBtn.disabled = false; if (attachmentPicker) attachmentPicker.setDisabled(false); }, 2000);
      } catch(e) {
        var st = c.querySelector('.post-status') || Object.assign(document.createElement('div'), { className: 'post-status' });
        st.textContent = '\u6295\u7a3f\u30a8\u30e9\u30fc: ' + String(e); st.style.color = 'var(--red)'; c.appendChild(st);
        newBtn.textContent = '\u6295\u7a3f'; newBtn.disabled = false;
        if (attachmentPicker) attachmentPicker.setDisabled(false);
      }
    });
  }
}
