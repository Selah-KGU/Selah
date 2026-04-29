// Thread Detail + Reply
function renderThreadDetail(data, threadUrl) {
  var c = document.getElementById('content');
  if (!data) { c.innerHTML = '<div class="error">\u30c7\u30fc\u30bf\u304c\u3042\u308a\u307e\u305b\u3093</div>'; return; }
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
  // Show posts if available from initial parse
  if (data.posts && data.posts.length) {
    h += '<div id="threadPostsArea"></div>';
  } else {
    h += '<div id="threadPostsArea"><div class="loading-text" style="text-align:center;padding:20px;color:var(--text-tertiary);font-size:13px">\u8aad\u307f\u8fbc\u307f\u4e2d...</div></div>';
  }
  h += '<div class="reply-section"><h3>\u8fd4\u4fe1</h3>';
  h += '<textarea id="replyContent" class="text-input" rows="4" placeholder="\u8fd4\u4fe1\u5185\u5bb9\u3092\u5165\u529b..."></textarea>';
  h += '<div class="reply-actions"><button id="replyBtn" class="btn primary">\u9001\u4fe1</button></div></div></div>';
  c.innerHTML = h;
  // Render posts
  if (data.posts && data.posts.length) {
    renderThreadPosts(document.getElementById('threadPostsArea'), data, threadUrl);
  }
  var replyBtn = document.getElementById('replyBtn'), replyContent = document.getElementById('replyContent');
  var replyDraftKey = lunaDraftKey(['discussion-reply', threadUrl, 'root']);
  bindDraftField(replyContent, replyDraftKey);
  replyBtn.addEventListener('click', async function() {
    var inv = window.__TAURI__?.core?.invoke;
    if (!inv || !replyContent.value.trim()) return;
    replyBtn.disabled = true; replyBtn.textContent = '\u9001\u4fe1\u4e2d...';
    try {
      var result = await inv('luna_reply_discussion', { url: threadUrl, content: replyContent.value.trim(), parentPostId: null });
      replyContent.value = '';
      clearDraftValue(replyDraftKey);
      replyBtn.innerHTML = ICONS.done + ' ' + escapeHtml(result);
      // Refresh posts after reply
      setTimeout(async function() {
        replyBtn.textContent = '\u9001\u4fe1'; replyBtn.disabled = false;
        try {
          var threadData = await inv('luna_fetch_thread_posts', { url: threadUrl });
          renderThreadPosts(document.getElementById('threadPostsArea'), threadData, threadUrl);
        } catch(e2) {}
      }, 1500);
    } catch(e) {
      var st = c.querySelector('.status') || Object.assign(document.createElement('div'), { className: 'status' });
      st.textContent = '\u6295\u7a3f\u30a8\u30e9\u30fc: ' + String(e); st.style.color = 'var(--red)'; c.appendChild(st);
      replyBtn.textContent = '\u9001\u4fe1'; replyBtn.disabled = false;
    }
  });
}
