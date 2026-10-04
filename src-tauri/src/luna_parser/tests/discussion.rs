use super::super::*;

#[test]
fn test_parse_thread_detail_extracts_discussion_file_attachments() {
    let html = r#"
        <form id="forumsPostFile" action="/lms/course/forums/thread_postfile">
          <input type="hidden" name="idnumber" value="202647210001">
          <input type="hidden" name="forumId" value="4721">
          <input type="hidden" name="threadId" value="222700">
          <input type="hidden" name="fileId" value="">
          <input type="hidden" name="fileName" value="">
        </form>
        <div class="contents-title-txt">生成AIと政治情報</div>
        <div id="threadPostListArea">
          <div class="clearfix">
            <div class="discussion-message-block">
              <div class="postUser">氏名:山田太郎</div>
              <div class="postDate">2026/04/29 10:12</div>
              <div class="postContentsText">資料を添付します。</div>
              <div class="postId contents-hidden">222720</div>
              <div class="discuss_mess_file">
                <span class="link-txt downloadFile">政治情報課題における生成AIの影響に関する一考察.pdf</span>
                <div class="contents-hidden fileName">政治情報課題における生成AIの影響に関する一考察.pdf</div>
                <div class="contents-hidden objectName">2026/47/21/b3/4721b302-3519-4bff-acdc-e5736fe8b6c2</div>
                <div class="contents-hidden postId">222720</div>
                <div class="contents-hidden scanStatus">1</div>
              </div>
            </div>
          </div>
        </div>
    "#;

    let result = parse_luna_thread_detail(html);
    assert_eq!(result.posts.len(), 1);
    let post = &result.posts[0];
    assert_eq!(post.thread_id, "222720");
    assert_eq!(post.attachments.len(), 1);
    let att = &post.attachments[0];
    assert_eq!(
        att.name,
        "政治情報課題における生成AIの影響に関する一考察.pdf"
    );
    assert_eq!(
        att.object_name,
        "2026/47/21/b3/4721b302-3519-4bff-acdc-e5736fe8b6c2"
    );
    assert_eq!(att.download_action, "/lms/course/forums/thread_postfile");
    assert!(att
        .download_params
        .iter()
        .any(|(k, v)| k == "fileId" && v == &att.object_name));
    assert!(att
        .download_params
        .iter()
        .any(|(k, v)| k == "fileName" && v == &att.name));
    assert!(att
        .download_params
        .iter()
        .any(|(k, v)| k == "postId" && v == "222720"));
    assert!(att
        .download_params
        .iter()
        .any(|(k, v)| k == "scanStatus" && v == "1"));
}

#[test]
fn test_parse_discussion_thread_falls_back_to_row_local_quill_payload() {
    let html = r#"
                    <div class="course-title-txt">日本語教育センター 51001004 日本語I 4</div>
                    <div class="contents-title-txt">掲示板 テーマトップ</div>
                    <div id="themeTopList">
                        <div class="result-list sp-contents-hidden">
                            <div class="theme-top-thread-title link-txt" onclick="viewthread(56777);">主題文（１）</div>
                            <div class="theme-top-thread-author">榎本 可奈子</div>
                            <div class="theme-top-thread-createdate">2026/05/27 16:05</div>
                            <div class="theme-top-thread-postzyoukyou">未読0件 / 既読13件 / 投稿数1件</div>
                            <script>
                                _QuillUtil.threadContents1.setJsonData("{\"ops\":[{\"insert\":\"ここには何も書かないでください。\\n\"}]}", 'reference');
                            </script>
                        </div>
                    </div>
            "#;

    let result = parse_luna_discussion_thread(html);
    assert_eq!(result.posts.len(), 1);
    assert_eq!(result.posts[0].thread_id, "56777");
    assert_eq!(result.posts[0].title, "主題文（１）");
    assert!(result.posts[0]
        .content
        .contains("ここには何も書かないでください。"));
}

#[test]
fn test_parse_discussion_thread_uses_global_one_based_quill_payloads() {
    let html = r#"
                    <div class="course-title-txt">日本語教育センター 51001004 日本語I 4</div>
                    <div class="contents-title-txt">掲示板 テーマトップ</div>
                    <div id="themeTopList">
                        <div class="result-list sp-contents-hidden">
                            <div class="theme-top-thread-title link-txt" onclick="viewthread(56777);">主題文（１）</div>
                            <div class="theme-top-thread-author">榎本 可奈子</div>
                            <div class="theme-top-thread-createdate">2026/05/27 16:05</div>
                            <div class="theme-top-thread-postzyoukyou">未読0件 / 既読13件 / 投稿数1件</div>
                        </div>
                    </div>
                    <script>
                        _QuillUtil.threadContents1.setJsonData("{\"ops\":[{\"insert\":\"テキストｐ60\\n\"}]}", 'reference');
                    </script>
            "#;

    let result = parse_luna_discussion_thread(html);
    assert_eq!(result.posts.len(), 1);
    assert_eq!(result.posts[0].title, "主題文（１）");
    assert!(result.posts[0].content.contains("テキストｐ60"));
}

#[test]
fn test_parse_discussion_thread_falls_back_when_row_container_differs() {
    let html = r#"
                    <div class="course-title-txt">国際学部 34134000 社会言語学基礎</div>
                    <div class="contents-title-txt">掲示板 テーマトップ</div>
                    <div id="themeTopList">
                        <div class="thread-entry-alt">
                            <div class="theme-top-thread-title link-txt" onclick="viewthread(59001);">第3回投稿課題</div>
                            <div class="theme-top-thread-author">田中 花子</div>
                            <div class="theme-top-thread-createdate">2026/06/08 15:40</div>
                            <div class="theme-top-thread-postzyoukyou">未読0件 / 既読0件 / 投稿数1件</div>
                        </div>
                    </div>
                    <script>
                        _QuillUtil.threadContents1.setJsonData("{\"ops\":[{\"insert\":\"ある方（Yes）: 具体例を書いてください。\\n\"}]}", 'reference');
                    </script>
            "#;

    let result = parse_luna_discussion_thread(html);
    assert_eq!(result.posts.len(), 1);
    assert_eq!(result.posts[0].thread_id, "59001");
    assert_eq!(result.posts[0].author, "田中 花子");
    assert!(result.posts[0].content.contains("具体例を書いてください。"));
}

#[test]
fn test_parse_discussion_thread_maps_one_based_quill_payloads_per_row() {
    // Multiple threads whose Quill variables are numbered from 1.
    // Each subtopic must render its own body, not the previous row's.
    let html = r#"
                    <div class="contents-title-txt">掲示板 テーマトップ</div>
                    <div id="themeTopList">
                        <div class="result-list sp-contents-hidden">
                            <div class="theme-top-thread-title link-txt" onclick="viewthread(101);">スレッドＡ</div>
                            <div class="theme-top-thread-author">榎本 可奈子</div>
                        </div>
                        <div class="result-list sp-contents-hidden">
                            <div class="theme-top-thread-title link-txt" onclick="viewthread(102);">スレッドＢ</div>
                            <div class="theme-top-thread-author">田中 花子</div>
                        </div>
                    </div>
                    <script>
                        _QuillUtil.threadContents1.setJsonData("{\"ops\":[{\"insert\":\"これはスレッドＡの本文です。\\n\"}]}", 'reference');
                        _QuillUtil.threadContents2.setJsonData("{\"ops\":[{\"insert\":\"これはスレッドＢの本文です。\\n\"}]}", 'reference');
                    </script>
            "#;

    let result = parse_luna_discussion_thread(html);
    assert_eq!(result.posts.len(), 2);
    assert_eq!(result.posts[0].title, "スレッドＡ");
    assert!(result.posts[0].content.contains("スレッドＡの本文"));
    assert_eq!(result.posts[1].title, "スレッドＢ");
    assert!(result.posts[1].content.contains("スレッドＢの本文"));
}

#[test]
fn test_parse_discussion_thread_dedupes_responsive_list_copies() {
    // LUNA emits the same threads twice for responsive layouts. The
    // broadened row selector matches both copies, so each thread must
    // still be emitted once, with its own (non-shifted) body.
    let html = r#"
                    <div class="contents-title-txt">掲示板 テーマトップ</div>
                    <div id="themeTopList">
                        <div class="result-list sp-contents-hidden">
                            <div class="theme-top-thread-title link-txt" onclick="viewthread(201);">スレッド甲</div>
                            <div class="theme-top-thread-author">榎本 可奈子</div>
                        </div>
                        <div class="result-list sp-contents-hidden">
                            <div class="theme-top-thread-title link-txt" onclick="viewthread(202);">スレッド乙</div>
                            <div class="theme-top-thread-author">田中 花子</div>
                        </div>
                        <div class="contents-result-list">
                            <div class="theme-top-thread-title link-txt" onclick="viewthread(201);">スレッド甲</div>
                            <div class="theme-top-thread-author">榎本 可奈子</div>
                        </div>
                        <div class="contents-result-list">
                            <div class="theme-top-thread-title link-txt" onclick="viewthread(202);">スレッド乙</div>
                            <div class="theme-top-thread-author">田中 花子</div>
                        </div>
                    </div>
                    <script>
                        _QuillUtil.threadContents1.setJsonData("{\"ops\":[{\"insert\":\"甲の本文\\n\"}]}", 'reference');
                        _QuillUtil.threadContents2.setJsonData("{\"ops\":[{\"insert\":\"乙の本文\\n\"}]}", 'reference');
                    </script>
            "#;

    let result = parse_luna_discussion_thread(html);
    assert_eq!(result.posts.len(), 2);
    assert_eq!(result.posts[0].thread_id, "201");
    assert!(result.posts[0].content.contains("甲の本文"));
    assert_eq!(result.posts[1].thread_id, "202");
    assert!(result.posts[1].content.contains("乙の本文"));
}
