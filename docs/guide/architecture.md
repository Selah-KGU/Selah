# アーキテクチャ

Selah の技術的なアーキテクチャの概要です。

## 全体構成

```
+------------------------------------------+
|            Selah (Tauri 2)               |
+------------------------------------------+
|  Frontend (WebView)                      |
|  +------------------------------------+  |
|  | Svelte 5 + TypeScript              |  |
|  | Vite (Build)                       |  |
|  +------------------------------------+  |
+------------------------------------------+
|  Backend (Rust)                          |
|  +------------------------------------+  |
|  | Tauri Commands                     |  |
|  | HTTP Client (reqwest)              |  |
|  | HTML Parser (scraper)              |  |
|  | SQLite (rusqlite, WAL)             |  |
|  | AI Client (Apple Intelligence /    |  |
|  |            OpenAI / Gemini)        |  |
|  | STT (sherpa-onnx + SenseVoice)     |  |
|  +------------------------------------+  |
+------------------------------------------+
|  External Services                       |
|  +------------------------------------+  |
|  | KWIC / Luna / Microsoft 365       |  |
|  | OpenAI API / Google Gemini API    |  |
|  | Open-Meteo API                    |  |
|  +------------------------------------+  |
+------------------------------------------+
```

## フロントエンドのリソース寿命

主画面・側欄 Agent と AI 設定は `ResourceScope` でネイティブ購読を所有し、ページ終了時にタイマーと描画要求も解除します。ページを破棄した時点でコールバックを無効にし、購読解除をまとめて行います。非同期の登録が破棄後に完了した場合も、その購読は直ちに解除します。解除の失敗が他のリソースの清掃を止めず、既に解除した購読を再度解除しません。設定の通知タイマーは新しい通知で置き換え、古いタイマーが後の読み込み表示を消さないようにします。

`Dashboard.svelte` はトレイの移動、Agent の表示、AI 設定変更、保存済み LIVE の TODO 提案の四つを並列登録します。これらは独立した通知なので、一つの登録失敗で他の成功済み購読を止めず、各ハンドルを scope で個別に所有します。ログアウトなどで Dashboard が終了すれば、登録中でも callback を無効にし、遅い解除ハンドルを直ちに一回だけ解放します。通知・メールの四つの cache 購読も同じ scope に含めます。読み取り済み ID の取得後の badge 再計算、onboarding の表示判定、遅いページ import の成功・失敗は終了後に反映しません。AI readiness の共有 store への既に開始済みの取得は従来どおり完了させ、終了した Dashboard から新しい取得は起動しません。

`tests/dashboard-lifecycle.test.mjs` の九項目は実際の Dashboard script を代替 transport・store・lifecycle 境界で実行し、全通知の同時登録、登録中の終了、遅いイベントと解除、onboarding と badge の遅い応答、ページ import、独立した登録失敗、解除例外、新しい Dashboard への古い callback の混入を検証します。活動中のトレイ移動、AI 設定更新、TODO の完全な内容と source path、通知の既読判定、メール未読数は従来のままです。これは購読の寿命と競合の検証であり、アプリの RSS・GPU 使用量や白画面の解消を測定したものではありません。

### 起動時の LIVE 判定と復旧の寿命

根の `App.svelte` は `liveHasActiveSession` / `live_has_active_session` で録音の存在だけを取得します。実際の返答本文は `true` の四 byte または `false` の五 byte で、転写・pending・要約・白板・講義情報を送信しません。ネイティブは blocking worker で LIVE の lock を待ち、存在を確認したら lock を解放して bool を JSON にします。`try_lock` が混雑していることを非活動と扱わず、STT lock、マイク、IO や snapshot revision の予約は行いません。既存の完全な記録取得・保存・surface API は保持します。デモは従来の保存済みデモ session の活動状態を返し、実アプリへの IPC を行いません。

logout 購読を起動の読み取りより先に登録し、`ResourceScope` が登録中の終了と遅い解除 handle を扱います。起動ごとに取った版と scope の活動状態が一致する間だけ、LIVE を開く、tray・foreground refresh を開始する、起動の完了状態を変える処理を行います。logout は demo module の import を待つ前に版を進め、tray と refresh を停止し、既存の service reset・cache invalidation・認証 latch の解除を行います。遅い LIVE 判定や大学 session の復旧が後で届いても、logout を取り消しません。

`restoreAllSessions` は省略可能な活動判定を受け取り、初期 disk read、secondary validation、headless sync、mail read の各待機後に確認します。古い結果から store の認証・mail・service 状態を再設定せず、validation 後に新しい headless sync を開始しません。既に発行済みのネイティブ要求を取り消す機構ではなく、現在の版だった時点で反映した結果も巻き戻しません。判定を渡さない呼び出しは従来どおり復旧します。

実際の根の script の七項目、実際の復旧関数の七項目、LIVE client の三項目は、登録中の終了、logout と遅い成功・失敗の競合、デモ、既存ユーザーの期限切れ badge、各復旧段階、大学の完全な identity、429 の既存の扱い、完全なデモ記録の保持を確認します。ネイティブの三項目は一万行の履歴が変わらず所有や revision を増やさないこと、繁忙 lock を worker で待ちながら async task が進むこと、poisoned lock を false とせずエラーにすることを確認します。この四・五 byte は JSON 本文だけの大きさであり、IPC 全体やアプリの RSS・GPU 使用量、白画面の再現・解消を測定した値ではありません。

### ログインページの登録・操作・タイマー

`Login.svelte` は成功・失敗・取消の三つの通知を `ResourceScope` と `acquireResourceGroup` で並列登録します。初期化とサインイン操作は登録中の同じ Promise を待ち、全ての handle が取得できるまでログインウィンドウを開きません。一つの登録失敗は group 全体の callback を無効にして、取得済み・遅れて到着する handle を解放します。失敗を既存の表示へ反映し、次のサインイン操作で再登録します。登録待ちとウィンドウを開く要求中の重複クリックは合流し、登録を繰り返したり複数の要求を発行したりしません。正常時の identity の全フィールド、成功後の foreground refresh、取消時の loading 解除、既存のエラー本文は保持します。

ページを終了した時点で通知と timer を無効にし、遅い window-open の失敗が別のページの認証を上書きしません。ロゴの連続クリックを数える三秒 timer は `ResourceScope.schedule` で所有し、置き換え・七回の確認表示・終了時に解除します。既にキューに入った古い timeout が新しいクリック数を消費しません。七回のクリックと確認・取消、Windows の既存のウィンドウ操作は保持します。デモの開始も要求中の重複を除き、`enterDemoMode` の省略可能な活動判定は module の import 後に再確認します。終了後はデモを有効化したり tray・refresh を再開したりしません。省略した既存 caller の処理順序は従来どおりです。

`tests/login-lifecycle.test.mjs` の十項目は実際の Login script を transport・store・timer・lifecycle 境界で動かします。三通知の同時登録と全登録前の操作抑止、百回の重複クリック、登録中の終了、失敗時の group 解放と再試行、完全な identity、遅いエラー、既存のエラー形式、旧 timer の配送、七回の確認、ウィンドウ操作、デモの開始、解除例外を検証します。`tests/demo-entry-lifecycle.test.mjs` の三項目は実際の `enterDemoMode` を使い、初めから終了した caller、module 読み込み中の終了、省略時と活動中の既存の処理順序を確認します。これはログインウィンドウ、資格情報、ネットワークや実際のデモを起動しない競合検証であり、アプリの CPU・RSS・GPU 使用量を測定したものではありません。

### Foreground refresh の所有と状態読み取り

周期更新は Rust が所有し、`BackgroundRefresh` は WebView の可視状態購読と復帰時の補完だけを所有します。通常の `startBackgroundPolling()` は活動中なら何もしません。ログイン identity と再ログイン core の完了は `startBackgroundPolling(true)` で補完を明示し、既に活動中でも古い状態読み取りを無効にして新しく取得します。終了・デモ移行では購読を解除し、解除前に配送された旧 callback も scope で無効にします。再開では foreground session の六十秒 cooldown を初期化します。

`CoalescedStatusRead` は AI、session、各 task の取得中の Promise を共有し、結果・失敗を版で区別します。停止時は進行中の読み取りを無効にし、次の所有者は旧 IO の終了を待たずに開始できます。後端の AI・session 通知は対応する読み取りを、cache 通知は該当 task の読み取りだけを無効にしてから通知を反映します。古い失敗はログや新しい状態を変更しません。手動 AI 更新の返答、key 指定時の局所結果、task の欠落・失敗時の null、各更新間隔と cache 名の alias は保持します。

task 表示の十四 timestamp は `get_backend_task_timestamps` の一回の IPC で取得します。以前の十三回の個別 cache timestamp と一回の完全な `get_schedule_snapshot` は不要です。原生の `Database::cache_timestamps` は blocking pool 上で一つの read transaction を使い、cache は covering index、schedule は `schedule_snapshot_state.updated_at` だけを読みます。JSON、本体の課程データ、AI 結果の読み込み・schedule の構築はしません。`BackendTaskStatusReader` が共有 batch から各 UI key へ戻し、task ごとの版も維持します。一つの通知は共有 batch の次回取得を許し、その task の古い結果だけを捨てます。まだ IPC を発行していない段階の個別通知も、他の task を null にしません。停止は未発行 batch を止め、発行済み結果は task の版で捨てます。

`tests/background-refresh.test.mjs` の二十六項目は製品の API 関数・通知 callback と共有制御を、代替 IPC・document・store・時計で実行します。百回の通常 start は十四 task の一 metadata batch と一 AI 読み取りに合流します。停止前後の遅い応答・失敗、古い callback、通知による個別の無効化、可視状態、cooldown の境界と再開、再入、デモ、手動更新、batch の失敗と再試行、欠落・非正の stamp、cache alias、microtask の順序を確認します。実際の Login script と再ログイン entry を補完制御につないだ検証では、活動中でも認証成功後に一回の新しい取得を行います。原生の三項目は一時 DB を使い、返答の key 順序と null、個別 timestamp との一致、covering index の query plan、二 MiB の不正な UTF-8 blob と schedule の JSON 列を読む必要がないこと、cache 本体を保持することを検証します。

既存の cache queue とその invalidation 世代、完全な課表表示の `get_schedule_snapshot` は変更しません。デモの手動 task 確認は従来の合成 schedule stamp をローカル取得し、native IO は発行しません。既に発行した native IO や app-lifetime の通知、手動コマンドを中止する処理ではなく、実アプリの CPU・RSS・GPU や LIVE 白画面への効果を測定したものでもありません。

### 再ログイン要求の合流と終了

`initiateRelogin` は同じ frontend module 内で進行中の Promise を共有します。所有する Promise を store 更新・native 登録の前に記録するため、複数の入口や store callback からの再入も新しい window 要求を増やしません。完了後の foreground refresh とエラー処理は一回だけ行い、全 caller に同じ結果を返します。要求を解放する `finally` は自身の Promise と一致する場合だけ slot を消します。デモの返答と expiry の解除は従来どおりです。

`visibleLogin.ts` の `waitForVisibleLogin` が四つの通知（identity、大学 core の完了、失敗、取消）を `ResourceScope` と購読 group で並列登録し、全登録後だけ window を開きます。Promise executor は同期で、非同期の初期化失敗も同じ終了処理へ送ります。終了時は callback を無効にして購読を解除し、進行中の表示を戻してから結果を解決します。登録中に完了・失敗・取消が届いても、遅い handle は直ちに一回だけ解除し、その後 window を開きません。一つの登録失敗は group 全体を解放します。古い window-open が後で失敗しても、終了済み scope から次の要求の表示や所有を変更しません。

identity 通知では全フィールドを既存の `setAuthFromSession` へ渡し、core の完了まで要求や refresh を終えません。core の二つの認証結果（片方が false の場合も含む）、取消の `__login_cancelled__`、失敗の `再ログインに失敗しました` と公開 caller の null 結果は保持します。購読解除の例外が残りの cleanup や Promise の解決を止めません。これは同一 WebView の要求をまとめる処理で、異なる WebView の native ログイン要求を排他するものではありません。

`tests/visible-login.test.mjs` の十一項目は実際の公開・内部 entry と製品の helper を使い、百要求の Promise と四購読・一 window・一 refresh、全登録前の待機、登録途中の三つの終了通知、失敗後の再試行、完全な identity と core 待機、既存の結果とエラー、旧 window 応答、解除例外、store 再入、デモを確認します。transport・store は代替境界で、実ウィンドウ、SSO、資格情報は使いません。全アプリの CPU・RSS・GPU や白画面への効果はこの検証では測定しません。

Agent の会話ストリームは `ResourceSlot` を使い、会話変更ごとに登録の世代を進めます。遅れて登録された旧会話の購読は破棄し、登録完了前に届いたイベントも世代で区別します。メッセージ・タイトル・テーマ・ページ文脈の読み取りは、ページの寿命や会話・文脈の変更を確認してから反映します。既存会話のメッセージを存在確認と表示用に二回読む処理を一回にまとめ、スクロールの描画要求は一フレームに一回だけ登録します。

側欄のページ文脈はバックエンドの `document-tabs-changed`（タブ切り替えとページ読み込み時にも発行）を優先します。900 ms の常時確認を、表示中のみ 30 秒ごとの復旧確認とフォーカス・可視状態の変更時の確認に置き換えます。確認中の要求は共有し、遅い応答がその間に受け取った新しいタブの状態を上書きしません。送信前にも文脈を確認します。これは確認の頻度を減らす実装であり、アプリ全体の CPU や GPU の改善率を測定した値ではありません。

文書ウィンドウのタブバーも `ResourceScope` と `acquireResourceGroup` で六つの通知購読を所有します。購読を登録してから初期状態を読み、一つの登録が失敗した場合は登録中のものを含めて全体を解除します。終了後に登録・読み取りが完了しても、購読やタイマーを残さず、DOM のテーマや表示状態へ反映しません。ファイル検索の遅延送信、コピー表示、ドラッグ後のクリック抑止も同じ寿命に含めます。

タブ一覧・テーマ・Agent の表示状態は `LatestViewRead` で版を管理し、新しい通知を古い応答や失敗で上書きしません。タブのドラッグ開始と操作の前後でも一覧読み取りを無効にします。一覧とテーマの要求は既存の `createCacheSyncQueue` で合流・直列化し、読み取り中に届いた確認要求は一回の後続読み取りへまとめます。タブ一覧の復旧間隔は従来の三秒を保持し、非表示ではタイマーを解除し、待機中の復旧読み取りも省略します。再表示では一回の補完読み取りを行います。明示的な操作完了後の読み取りと、非表示中の通知による更新は続けます。

`tests/document-tabs.test.mjs` の十三項目は実際のコンポーネントの TypeScript と共有制御を、代替 IPC・タイマー・DOM 境界で検証します。終了後の登録、登録失敗、古い一覧・テーマ・Agent 表示の応答、ドラッグと操作の順序、非表示からの復帰、失敗後の再試行を確認します。取得中の百件の確認要求は一回の後続読み取りに合流し、同時実行数は一件です。これは状態・要求数・寿命の検証であり、Svelte の実 DOM 表示、実機の IPC 速度や WebKit GPU 障害の解消を測ったものではありません。

タブバーの操作は発行元の `tabId` を `document_tabs_send_control` へ渡します。ファイル検索の 140 ms の待機中に別タブへ移動した場合は予約を解除し、IPC の配送中に切り替わった場合も発行元以外へ転送しません。発行元が既に閉じていれば何も送信しません。ID を省略する既存の呼び出しは活動中のタブを使います。外部ブラウザーへ渡す URL と取得失敗時の代替 URL もクリック時のページから取り、ページ終了後は後続の起動を行いません。ナビゲーションの失敗とコピー表示は要求元の表示にだけ反映します。`ResourceScope.schedule` は一回のタイマーについても解除済み・実行済みの callback を無効にし、古い callback が置き換え後の検索や表示を消費しません。

`tests/document-tab-actions.test.mjs` の十項目は実際の操作処理を読み、表示境界として選択中のタブを明示的に与え、コンポーネント自身の effect を実行します。A → B → A、同じタブの等価更新、検索の連続入力、メニュー座標と payload、外部起動の成功・失敗・終了、古いナビゲーションの失敗、コピー中と完了後の表示切替を検証します。IPC・クリップボードは代替で、実際のファイル変更、ブラウザー起動や OS のクリップボードは操作しません。共有タイマーのテストは解除後の古い callback を直接実行して、親・子 scope の終了後も動かないことを確認します。

原生側の `document_tabs/state.rs` は所有ウィンドウから必要なタブを選んで一回だけ複製し、一覧は `DocumentTabInfo` の射影だけを作ります。以前は活動中のタブを一つ取る際にもウィンドウ全体を複製し、一覧では全体の複製後に controls・reopen などをもう一度複製していました。戻り値は引き続き所有するスナップショットで、通知の JSON 化と配送はロックの外です。四項目の Rust テストは選択元 ID、閉じた ID、既存の活動中選択、後の状態変更からの独立性、および 0/1/4/32 タブの一覧 JSON の完全一致を検証します。

`cargo test --offline --manifest-path src-tauri/Cargo.toml --lib document_tabs::state::tests::benchmark_tab_state_reads -- --ignored --nocapture` は、この実際の選択・射影と凍結した従来の複製経路を比較します。各タブに八 controls と JSON payload、子・孫 pane を持つ合成データを使い、百回の warmup 後、千回の取得を九回、前後の順序を交互にして測ります。通常の test profile は最適化なしです。読み取りと複製・破棄だけの局所比較で、Mutex、通知の JSON 化、IPC・描画・録音・GPU・RSS は含めません。

`document-tabs-changed` は `document_tabs/events.rs` で一覧を一回だけ取得し、その同じ snapshot の target からタブバー・Agent・split divider の対象集合を作り、一回の `emit_filter` で配送します。従来の target ごとの `emit_to` は、Tauri の `Any` 購読へも毎回届くため、全局の購読が一つの更新を `2 + 2 × タブ数` 回受け取っていました。現在は既存の label 指定購読の対象と payload を保持し、全局購読も一回だけ受け取ります。集合には従来と同じ二つの固定 label と各タブの二つの divider label を含め、JSON 化と配送を繰り返しません。

`document_tabs_set_controls` は同じ所有ウィンドウのロック内で target の解決・controls の比較・更新を行い、実際の変更があった場合だけ通知します。明示的な target がある報告も含め、活動中タブの不要な複製を除去します。id・label・target の既存 alias、ID 省略時の活動中タブ、不明 target のエラーは保持します。controls の全十三フィールドと配列の順序を比較し、ボタンの有効状態、indicator、深い JSON payload や空配列への変更も通知します。

`document_tabs/events_tests.rs` の三項目は `tauri::test::mock_app` の実際のイベント機構を使います。凍結した旧配送と新配送の全局 callback 数は、0/1/4/32 タブでそれぞれ 2/4/10/66 → 1 です。全 payload の JSON 一致、label 指定購読の旧・新の受信範囲、対象外 label と App 限定購読への不配信も検証します。Window・Webview・WebviewWindow・AnyLabel のフィルター条件は製品の関数で確認します。state の追加四項目は同一 controls の千回報告が通知を要求しないこと、全フィールドと順序の変更、解除、alias と既存エラーを検証します。Tauri の test feature は dev dependency に追加しています。これらは代替 runtime と合成データの検証で、実際の OS ウィンドウや WebKit、録音は開始しません。

主画面の Copilot dock は通知の登録を完了してから初期一覧を読みます。`ResourceScope`、購読 group、`LatestViewRead` と読み取り queue で、終了・登録失敗後の古い callback を無効にし、遅い一覧応答が新しい通知を上書きしません。再表示で一回補完し、登録に失敗していれば再試行します。登録中の可視状態変更は初期購読の完了を待ち、同時登録を増やしません。定期 poll は追加せず、非表示中のイベント更新と既存の animation の停止条件を保持します。`tests/copilot-dock.test.mjs` の十一項目は実際の script を代替 IPC・表示境界で動かし、読み取りと登録の順序、通知との競合、終了、可視状態、百件の要求の合流、他 owner、失敗後の復旧と登録の再試行、閉じる操作の `focus: false` を検証します。

Files surface は `ResourceScope` と購読 group で theme、course focus、target を限定した toolbar control を所有し、購読後に初期一覧を取得します。theme IO はファイル一覧を遅らせず、通知が旧 theme 応答を無効にします。最初の一覧取得中の course focus は保留し、実際の course label に対する既存の大小文字を無視する照合を保持します。終了や登録失敗の古い callback は無効にし、遅れて取得した解除 handle も一回だけ解放します。削除 hint の 2.5 秒 timer と IntersectionObserver の遅い配送もページの寿命で扱います。

一覧は `LatestViewRead` と読み取り queue で合流し、百回の同時要求でも一回の進行中の取得と一回の補完取得にまとめます。履歴変更・削除・directory scan・重複整理を直列化し、それらが古い一覧を無効にします。操作中の一覧要求は完了後に補完し、旧一覧が削除済みの項目を画面へ戻しません。現在の一覧取得の失敗では既存の行と選択を保持し、エラーを表示します。批量操作は送信した ID と path を捕捉し、応答後の選択で対象を差し替えません。元の二回クリックによる削除確認を保持し、送信後に選択した項目を解除しません。終了後の操作結果は状態と toolbar へ反映せず、追加の一覧・重複 scan も起動しません。ネイティブで開始済みの操作自体を取り消す仕組みではありません。

画像・テキスト preview の IO・待機 queue・可視 consumer は `filePreviewController.ts` に集約します。同じ履歴版の要求は共有し、最大四件の開始済み IO を保持します。`weightedLruCache.ts` を Markdown の `RenderedTextCache` と共用し、preview の再利用 cache は最大百二十八件・三十二 MiB の文字列保持量の見積もりで制限します。見積もりは key、kind、mime、data URL、text の UTF-16 長であり、JS object・実際の heap/RSS・decode した image・DOM/GPU を含みません。cache に入らない大きい preview も現在の consumer へ全文を表示し、同じ可視 preview の consumer 間で共有します。そのため可視領域と百六十 px の前後領域の値は cache の上限とは別に保持し、ページ全体の三十二 MiB 上限とは扱いません。

`IntersectionObserver` は取得後も target を観測し、領域を離れた時、filter で node が破棄された時、list view へ移った時に display map の項目と `<img>` を除去します。開始前の IO は取り消し、開始済みの IO は実際の終了まで四件の枠に数え、領域外の完了を display map へ再公開しません。完了した同じ版の値は上限内で再利用します。履歴 key は path・ID・size・download 時刻・存在状態を含み、新しい記録の版と除去した記録の古い IO が画像を戻さないようにします。filesystem の外部変更を監視する仕組みではありません。preview node の再利用では [takeRecords](https://developer.mozilla.org/en-US/docs/Web/API/IntersectionObserver/takeRecords) で旧配送を排出し、別 target の保留通知を失わず処理します。list → icons が一回の Svelte tick に入る場合と履歴 A → B → A でも観測を登録し直します。更新は同じ reactive map の版 key 一件だけを操作します。ページ終了時は consumer・待機 queue・cache・map・observer を解放し、遅い成功・失敗は保存しません。画像の解像度・動画像の形式・元 byte 列・text の内容は変更しません。

`get_download_preview` は path 検証、filesystem の metadata と read、base64 と応答 JSON を既存の `background_ipc::respond` の blocking worker に移します。画像は元の全 byte と mime・data URL、text は trim した空でない行を LF で繋いだ最初の七百 Unicode scalar を返し、10 MiB / 512 KiB の既存のサイズ判定と unsupported / 空の null 応答を保持します。text は全行を中間 string に結合せず、preview の prefix が完成した時点で結合を止めます。元の file read と UTF-8 の lossy 変換は保持します。

`tests/files-surface.test.mjs` の二十四項目は実際の script と queue・resource helper を代替 IPC、DOM・timer・observer 境界で検証し、終了、登録失敗、選択と読み取りの競合、百件の合流、preview の四件上限と同じ map の保持、list / icons 切替、閉じた重複 modal を確認します。Rust の四項目は 4,272 個の Unicode / whitespace / 七百文字境界の旧関数との一致、最大 text 入力、全 image byte と mime・拡張子、UTF-8 とサイズ判定・unsupported / 空 / IO error を fixture で比較します。text 出力の capacity は最大 2,800 bytes の prefix の予約で、アプリ全体の allocation 数や RSS の測定値ではありません。

`node scripts/check-files-surface-browser.mjs` は実際の Svelte FilesSurface と IntersectionObserver、image decode を使います。2026-10-08 の browser で二十三項目が通過し、同じ DOM node の版変更による preview の再取得と選択の保持、削除の二段階確認と履歴更新だけが失敗した時の表示、reactive map の image / text 更新、cache 再利用、検索・course focus・欠落表示、提出した履歴項目の除去と後続の選択の保持、重複 modal、theme、unmount 後の解除を検証しました。transport とファイルは memory fixture に置き換えており、実ファイルを削除・共有せず、インストール済みアプリや録音も起動しません。この検証はファイルページの正確性と UI から離した処理の確認であり、LIVE の GPU 障害を再現・解決した結果ではありません。

`tests/file-preview-controller.test.mjs` の八項目は共有 IO と consumer の参照、待機百件の取消、cache を超える全文、五百件の traversal、版 A → B → A、list 切替と同じ版の進行中 IO、終了後の成功・失敗と全文の key / weight を確認します。`weighted-lru-cache.test.mjs` の三項目は count / byte 上限と LRU、超過・不正 weight、古い同 key の置換、null / 空値を確認し、既存 Markdown renderer の七項目も共通 cache へ移した後に通過します。component 側では継続する viewport 観測、二つの node の共有、領域外の待機取消と完了、履歴版による同一 node の再利用、保留観測の排出・履歴除去・同 tick の復帰を検証します。変更前の実際の script では継続観測と大きい preview の cache 上限の二項目が失敗しました。

`node scripts/benchmark-files-preview-browser.mjs --before` と option なしは、同じ現在の FilesSurface の UI に、変更前の preview・view 切替・破棄の処理と新処理をそれぞれ読み込みます。旧処理は `tests/fixtures/files-preview-before.json` に固定し、製品に含めません。九十六件の PNG fixture を作り、実 DOM、scroll、IntersectionObserver、image decode を使って directory 全体を巡回します。PNG の後ろに五百十二 KiB の補助 byte を足して大きい transport / string を作り、巨大な decoded image を用意せず一 px の画像として decode します。全 data URL の一致、全件への到達、短い scroll back、cache 内の再利用と容量を超えて退避された値だけの再取得、list 切替・unmount を確認します。transport とファイルは fixture であり、実際のダウンロード・録音・インストール済みアプリを操作しません。

2026-10-08 の macOS browser の viewport は両方 1,064 × 693 px で、旧十五・新十八の check が通過しました。同じ九十六件を読み終えた最下部の画像 DOM / display entry は九十六 → 二十六、cache は九十六件・134,248,088 bytes → 二十三件・32,165,224 bytes でした。cache と可視 consumer の同 key を重複計数しない文字列保持量の見積もりは 134,248,088 → 36,360,688 bytes、list view の display entry は九十六 → ゼロでした。可視値が cache 上限を超えるため、list 切替後には三件を再取得し、残りを再利用しました。最大の sample は新方式でも 58,736,496 bytes の文字列保持量の見積もりで、cache の三十二 MiB をページ全体の上限や RSS に換算しません。順に一回ずつ巡回する局所的な比較で、実際の GPU surface・画像の decompression や製品全体の memory / 速度 / 白画面の解消率は測定しません。

ダウンロード履歴は既存の `download_history.json` を保持し、`commands/downloads/history_store.rs` に読み取り・変更・保存を集約します。変更前は各操作が独立した snapshot を読み、整ファイルを直接上書きしていたため、同時ダウンロードで片方の記録を失い、古い scan が削除済みの記録を戻す場合がありました。新しい更新は固定の `download_history.lock` に独立した file handle で排他 lock を取り、その間に最新の JSON を読み、変更し、commit します。読み取りには共有 lock を使います。原子的 rename で inode が変わる JSON 自体を lock せず、sidecar は unlink しません。標準ライブラリの [File lock](https://doc.rust-lang.org/std/fs/struct.File.html#method.lock) を使い、同じ規約を使う thread と process 間を直列化します。旧バージョンや lock を無視する外部 writer との排他は保証しません。

`atomic_file.rs` は既存 LIVE の完全ファイル置換を共有化したものです。同じ directory に独自の一時ファイルを作り、64 KiB の buffer へ JSON を直接 encode し、flush と file sync 後に rename します。中間の全文 JSON string を作りません。通常の失敗と Rust の unwind では元の完全ファイルを保持し、未完の staging を除去します。process 強制終了時の staging cleanup や停電時の directory sync を保証する仕組みではありません。存在しない履歴だけを空配列として扱い、JSON 破損・読み取り失敗は error を返して普通の変更を停止します。ユーザーによる明示的な履歴 clear は破損した JSON も空配列へ置き換えます。ファイルの path・source・時刻・course・既存の五百件上限と後方互換の default field を保持し、アプリが長期保持する履歴 cache は追加しません。

新規 download、scan の最終 merge、ID / path の削除、course 名・移動 path・重複の migration が同じ transaction を使います。scan の directory walk と重複 file の SHA-256、物理的な削除・移動は履歴 lock の外です。scan は最新履歴と path を照合し、先に完了した download の ID と metadata を優先します。migration は file move 全体を含む filesystem transaction ではありません。bulk deletion は成功した canonical path だけをまとめ、従来の成功した file ごとの整履歴書き込みを、一回の履歴更新へ減らします。空対象・変更のない削除や migration は保存しません。file 削除の件数と履歴保存の error を区別し、保存 error もページと重複 modal に表示します。

一覧・既存 download の照合・directory / duplicate scan・削除と clear、download 設定の取得・保存は非同期 command から既存 `background_ipc` の blocking worker を使います。大きい応答の JSON encode も worker に置き、フロントエンドが受け取る配列・map・削除件数 object と null の形式を保持します。native 内部の typed snapshot 読み取りは error を伝え、任意の LIVE note を TODO 分析へ足す経路だけは error を log して note を省略します。

Rust fixture は旧 load/save の順序を固定して更新の消失と削除した行の復活を確認し、新 transaction と比較します。八 thread の二百件の完全な記録と lock を取らない外部 reader の JSON、二つの実子 process と parent の二十五件・競合する lock、panic 後の再更新、破損と IO failure、no-op の inode / mtime、実 directory scan、download と scan の metadata の優先順位、履歴上限・照合の alias、migration の course 境界と生存 file の優先を確認します。別の fixture が SHA-256 と保持推奨の既存順序、成功二件・失敗三件の bulk deletion と一回の履歴更新、保存だけの failure を検証し、共有 atomic writer の部分書き込みと unwind も検証します。current-thread async の test は OS lock を待つ worker と executor の継続、全文 Unicode を含む raw JSON を確認します。全て専用の一時 directory と test 用 process で実行し、実際の download directory、インストール済みアプリ、マイクは操作しません。Windows の実機動作や LIVE の GPU 障害を再現した検証ではありません。

Markdown reader は、文書と toolbar control の通知を自身の webview target に限定し、四組の購読を登録してから startup payload を読みます。theme の読み取りは文書の配達を待たせず、theme の通知が旧応答を無効にします。`ResourceScope` と購読 group は登録中の終了・登録失敗・遅い解除を扱い、startup の 300/700/1500 ms retry と toast も所有します。文書通知で startup retry を取り消すため、既に届いた文書を後の null 応答が「未配達」に戻しません。終了後の通知、読み取り、解析、保存結果は画面と toolbar へ反映しません。

ネイティブの `commands/downloads/markdown_delivery.rs` は target ごとに一つの delivery を保持し、再オープン時に旧 delivery を無効にします。全文は `Arc<Value>` で共有し、保留表・最初の通知・1.2 秒後の retry 用に大きな文字列を複製しません。startup の pull または版の一致する `ack_markdown_payload` で保留を解除し、以後の retry を停止します。タブを閉じる時も保留を解除し、遅い file read は公開しません。タブ存在確認と delivery の予約はタブ削除と同じ lock で直列化し、IO とイベントの JSON 化は lock の外で行います。開始済みの filesystem read 自体は中断しません。隠すだけのウィンドウ操作は既存のタブを保持します。startup 応答は既存の `background_ipc::respond` で全文の JSON を blocking worker で生成し、オブジェクトまたは null の既存形式で返します。

通知の `deliveryRevision` は u64 の十進文字列で渡し、reader は `BigInt` で比較します。同じ版の retry と古い版は再解析・スクロール初期化を行わず、新しい版では同じ本文でもファイルを再度開けます。ローカル保存は実際に提出した path・本文と文書の世代を捕捉し、IO 中に追加した入力を未保存の編集として残します。保存中に文書が再オープンされた場合、旧保存の成功・失敗で新しい表示を変更しません。共有は保存後にも未保存入力と文書の世代を確認します。Markdown の全文、DOMPurify の設定、structured / legacy whiteboard の分割は保持し、解析失敗は reader 内で表示します。後続の読み取りエラーも進行中の旧描画を無効にします。目次は別々の Markdown segment と whiteboard wrapper の `offsetParent` に依存せず、見出しの viewport 座標を scroll container の座標へ変換します。

`tests/markdown-reader.test.mjs` の十六項目は実際の reader script、Marked と白板 splitter を代替 IPC・timer・表示境界で動かし、重複、乱序、終了、retry、theme、保存中の入力、共有の拒否、解析エラーと完全な本文・白板参照を検証します。ここでは DOMPurify と DOM は境界で、sanitize と layout の検証には使いません。ネイティブの八項目は版と ACK、close・再予約と並行 read、保留解除、Arc の同一性、ほぼ 8 MiB の Unicode 本文の完全な JSON 往復と実際の startup command の raw response / null を確認します。

`node scripts/check-markdown-reader-browser.mjs` の localhost ページは実際の reader・Marked・DOMPurify・Svelte DOM と白板 renderer を使います。2026-10-08 のブラウザー検証では二十二項目が通過し、sanitize、重複見出し、白板後の目次ジャンプ、リンクと画像 preview、重複配達時の DOM / 白板の保持、保存中の入力、旧 retry、再オープン・エラー復旧と unmount 後の解除を確認しました。ネイティブ transport は fixture に置き換え、実ファイルへの書き込み・共有 sheet・インストール済みアプリ・録音は起動しません。これらは reader の正確性と所有権の検証で、アプリ全体の速度・RSS・GPU 障害や LIVE 白画面の解消率を測定した結果ではありません。

主画面・側欄 Agent の会話表示は `AgentConversationView` がメッセージ読み取りと流れる回答の購読をまとめて管理します。会話 ID が同じでも「A → B → A」の古い処理を世代で区別します。送信は履歴読み取りと購読の両方が完了してから開始し、準備中の重複送信を抑止します。終端イベント欠落後の復旧読み取りにも版を付け、新しい送信で無効にするため、同じ会話の新しい回答を古い履歴で消しません。読み取り・購読の失敗は再試行でき、ページ終了後の結果を画面へ反映しません。

共有会話の変更イベントは読み取りの無効化通知として扱い、通知内の ID ではなく DB に残る現在の共有会話を確認します。主画面は `LatestViewRead`、側欄は会話読み取りの世代で古い結果を拒否し、受け取った変更を再びネイティブへ書き戻しません。主画面の選択・新規作成・削除は待機中の共有読み取りを無効にします。側欄は既に購読済みの同じ会話なら、履歴・回答ストリームをリセットしません。読み取り失敗を空の共有選択として扱わず、エラーを返します。初期の共有会話読み取りはユーザーの選択・新規作成の意図を上書きせず、途中で無関係な会話が削除された場合は DB を再確認します。一覧更新には既存の `createCacheSyncQueue` を使い、同時イベントを一回の読み取りへまとめ、取得中の変更は後続の読み取りで確認します。主画面で使っていなかった AI 設定の初期読み取りも除去します。Agent の音声入力は画面が開始した入力 ID だけを扱い、初期表示で他の画面のマイクを引き継ぎません。状態の復旧読み取りは、その間に受け取った状態イベントを上書きしません。

`agent-conversation-view.test.mjs` は実際の会話表示制御を代替 IPC で動かし、履歴と購読の完了待ち、乱序応答、同じ ID への再訪、ページ終了、復旧と新しい回答の競合、削除、現在と過去の失敗、購読の再試行、共有変更の書き戻し抑止、削除後の遅い登録・履歴・通知、別会話の削除後も新しい回答の購読と復旧が有効であることを確認します。これらは画面の状態と購読寿命の検証であり、実際の録音の所有権や WebKit GPU 障害が解消したことを示す検証ではありません。

側欄も履歴の読み取り・回答の購読・完了後の復旧を共通制御へ移し、側欄専用の購読管理と履歴回読を除去しました。共有会話の起動読み取りが進行中なら送信準備も同じ Promise を待ち、追加の会話作成・履歴取得を開始しません。タイトルの読み取りは表示用であり、履歴と購読が準備できた送信を待たせません。送信準備は最初の await 前に一回分を予約し、ページ文脈の読み取りも入力を消費する前の準備に含め、準備中の停止は本文・添付を保持し、未提出のユーザーメッセージを表示へ追加せず、推論を開始しません。準備が失敗した場合も本文・画像を消費せず、同じ会話の購読失敗を再試行できます。別の会話へ切り替えた場合は古い準備を取り消しますが、同じ ID の購読の再登録は切り替えとは区別します。会話切り替え時は先に旧表示を解除するため、新会話の履歴取得に失敗しても旧会話のメッセージを新会話として表示しません。送信ごとの版と会話表示の世代で古い RPC の完了・失敗を拒否し、同じ会話で新しく送信する前に古い履歴の復旧を無効にします。待機中に編集された下書きは保持し、送信対象に含まれる添付だけを現在の配列から取り除きます。

`agent-panel-conversation.test.mjs` は側欄の実際の TypeScript 処理をコンポーネントから読み、IPC・DOM・Svelte の表示境界を代替して動かします。ID 到着前の削除、遅い履歴・失敗、重複通知中の回答保持、認識済みの下書き・添付の保持、空会話の自動作成抑止、購読失敗の再試行と終了後の遅い登録の解除を確認します。`agent-panel-send.test.mjs` は同じ実装を使い、起動中の送信の共有、重複送信の抑止、準備の取消・失敗・再試行、待機中の本文・添付の編集、同一会話と A → B → A の遅い RPC、旧完了回読と新しい送信、切り替えと終了中の送信抑止、ページ文脈の待機中の取消、遅いタイトルが送信を待たせないことを確認します。Svelte の DOM 描画・反応性や実機のマイクは検証しません。

`resource-scope.test.mjs` は遅延したネイティブ登録・会話切り替え・解除失敗と仮想タイマーを使い、破棄後のイベント無効化、旧購読の解除、タイマーの置き換えを検証します。実際の WebKit 障害の再現はこのテストには含めません。

側欄 Agent の流れる回答は `TextStreamBuffer` で約 48 ms ごとにまとめて描画します。各 token の全文再解析と各途中経過のキャッシュを行いません。終了・エラー・コンテキスト上限の処理前に残りの文字をすぐ反映し、会話切り替えやページ終了時は過去の未表示バッチを破棄します。途中も Markdown の表示と DOMPurify による処理を保ちます。主画面 Agent の既存のテキスト表示とバッチ更新はそのまま使います。

確定した回答・LIVE 要約は `RenderedTextCache` で LRU キャッシュします。Agent は最大 256 項目・8 MiB、LIVE は最大 128 項目・4 MiB を上限とし、Markdown 元文と処理後 HTML の UTF-16 文字列から保持量を見積もります。この値は JS オブジェクトや DOM を含む実際の RSS ではありません。単独で上限を超える回答も全文表示しますが、キャッシュへは保持しません。流れる途中の回答はキャッシュを通さず表示し、Agent の会話切り替えでは古い会話のキャッシュを除去します。

`markdownRenderer.ts` は専用の Marked インスタンスを作り、改行・GFM の設定が文書リーダーなどの共有 parser に波及しないようにします。キャッシュには DOMPurify の結果だけを入れ、途中表示にも同じ処理を使います。回帰テストは多量の token、終了時の末尾、Unicode・改行・Markdown の最終一致、キャッシュ容量と LRU、共有 parser からの設定分離を確認します。

`node scripts/benchmark-markdown-stream.mjs` は実際の Markdown renderer とストリームバッファを Node で読み込み、1000 個のテキスト片を仮想の 1 ms 間隔で渡します。旧方式に相当する各途中経過の解析・256 項目保持と、48 ms のバッチ・途中経過を保持しない方式を比較し、最終テキストと HTML の一致を確認します。ウォームアップ後に実行順を交互にした 5 回の中央値を使います。

2026-10-07 のローカル測定は旧方式 7430.083 ms / 1000 回の解析、現方式 132.983 ms / 21 回の解析でした。途中経過のキャッシュは旧方式 256 項目・文字列保持量の見積もり 24,456,960 bytes、現方式は 0 項目でした。これは Node 内の Markdown 解析とキャッシュ・バッファ処理の比較であり、DOMPurify、ブラウザのレイアウト、実際のアプリ RSS や GPU 障害の改善率を測定した値ではありません。

主画面 Agent の `appendAssistantText` は流れる回答の同じ行の reactive な content だけを更新します。従来は 48 ms のバッチごとに末尾の message を新しい object に置き換え、キー付きリストが全履歴の ID を再確認していました。現在は行と履歴配列の末尾 entry を保持し、本文を読む表示だけを更新します。新しい行の追加と終了時の確定は既存の経路を使います。既存の全文・ID・時刻・role・画像と Markdown 表示を保持し、履歴を切り詰めません。引用返信はクリック時の message の浅い snapshot を保持するため、同じ行の後続の本文更新が引用内容へ混ざりません。

`tests/agent-stream-message.test.mjs` は実際の主画面 script を使い、千個の Unicode chunk の完全な連結、流れる行の同じ参照と metadata、過去の user / assistant の不変性、終了時の全文と引用した prefix の保持を確認します。transport と DOM は境界に置き、native 操作を発行しません。会話切替・送信・終了イベントの所有権は既存の component テストでも検証します。

`node scripts/benchmark-agent-stream-browser.mjs --before` と同じコマンドの option なしは、旧・新の実際の append / quote 関数と現在の message markup を隔離した Svelte client に読み込みます。旧関数は `tests/fixtures/agent-stream-before.txt` に固定し、製品には含めません。各 URL を開くと、0・100・1000 履歴の同じ内容を生成し、二十回の更新で warmup した後、百バッチを九回測定します。ID getter の計数、既存 renderer の呼出回数、Markdown 解析回数を別に記録し、全文と実 DOM の文字列、後続 chunk が引用を変更しないことを確認します。旧・新は別のページで順に実行し、交互の測定ではありません。サーバーは Ctrl-C で停止します。

2026-10-08 の macOS 内ブラウザーでは、九回・合計九百バッチの履歴 ID 読取は、100 履歴が 180,000 → 0、1000 履歴が 1,800,000 → 0 でした。履歴 Markdown の再描画関数呼出と再解析は以前も現在もゼロで、重複するキー確認が対象です。百バッチあたりの中央値は 0 履歴が 1.700 → 0.700 ms、100 履歴が 7.200 → 0.600 ms、1000 履歴が 52.300 → 1.800 ms でした。計数 getter の負荷も含む局所的な状態更新・DOM の文字列更新と Svelte tick の合成比較です。履歴生成・初期 mount・最初の新規行・引用と終了操作は時間計測の外で、製品 CSS・レイアウト・スクロール・48 ms の実時間・IPC・モデル・マイク・アプリ全体の RSS/GPU は含めません。アプリ全体の速度改善率や白画面の解消率には換算しません。

LIVE も `ResourceScope` で常駐イベント・時間割キャッシュ通知・ウィンドウのフォーカス通知を所有します。初期化は同期の `onMount` から非同期処理を起動し、購読はセッション復旧や時間割・AI の読み取りより先に並列登録します。終了後の登録は解除し、描画要求も取り消します。

動的な STT の五組の購読は `ResourceSlot` の登録用子スコープと `acquireResourceGroup` でまとめます。登録中に非表示・終了・再登録が起きても旧コールバックを直ちに無効にし、取得済みの解除ハンドルを待たずに解放します。まだ返っていないハンドルは完了時に解放します。一つの登録失敗でも全体を無効にし、残りの登録が終了後に生き残りません。子スコープは親の終了でも閉じ、自分を閉じると親の所有表から外れます。

`LatestViewRead` は LIVE の時間割、AI 準備、音声状態、授業履歴プレビューを扱います。新しい読み取り、通知による更新、選択変更、ページ終了の後で古い結果や古いエラーを表示しません。AI の準備確認が設定変更で古くなった場合は、開始前に現在の結果を確認します。音声状態は現在の録音 ID と状態イベントを優先します。保存・要約など既に要求したバックエンド処理は継続し、終了した画面への通知や TODO への自動移動を抑止します。

授業履歴のプレビューは授業の識別子・名称・日付を派生キーにします。毎分の時間割再計算で同じ授業オブジェクトが作り直されても、保存済み字幕全体を読み直しません。自由ノートへの切り替えや読み取り中の選択変更でも旧プレビューを適用せず、開始・保存中のセッションを授業履歴で上書きしません。

子スコープと購読グループの回帰テストは登録途中の失敗・解除・終了と遅延したハンドルを検証します。読み取りの回帰テストは A → B → A、通知による無効化、終了後の成功と失敗、旧エラーと新しい準備状態の競合を検証します。これらは UI の購読・読み取り寿命の検証で、実機の録音や WebKit GPU 障害を再現した結果ではありません。

## SSO 認証連携

Selah は内蔵 WebView を用いた SSO セッション共有方式で認証を処理します。

1. 内蔵 WebView (WKWebView / WebView2) で関学 SSO のログイン画面を表示
2. ユーザーが SSO でログイン
3. WebView の認証セッションをネイティブ HTTP クライアント (`reqwest`) と共有
4. KG-Course・Luna・KWIC の 3 系統を一度のログインで認証

## データフロー

```
SSO Login
    |
    v
SSO Session --> reqwest HTTP Client
    |                    |
    +-----> KWIC API     |
    +-----> Luna API     |
    +-----> Mail API     |
                         v
                   HTML Parsing / JSON Parse
                         |
                         v
                   SQLite Cache (WAL)
                         |
                         v
                   Tauri Commands (IPC)
                         |
                         v
                   Svelte Frontend
```

## ローカルキャッシュ戦略

- **SWR (Stale-While-Revalidate)** 方式を採用
- 起動時はキャッシュデータを即座に表示し、バックグラウンドで最新データを取得
- ネットワーク不通時はキャッシュデータでフォールバック
- SQLite の WAL (Write-Ahead Logging) モードで読み書きの並行処理を実現

### 内容の変更と取得時刻

`data_cache.updated_at` は取得の鮮度、`revision` は内容の版を表します。秒単位の取得時刻だけでは同じ秒の変更を区別できないため、同期時の一致判定には永続的な版を使います。SQLite トリガーが各書き込みを追跡し、削除後に再作成した行にも新しい版を割り当てます。既存データへの追加マイグレーションで導入し、キャッシュを削除しません。

ウィンドウの再表示と `backend-cache-updated` は `backendCacheSync.ts` に集約します。`cacheSyncQueue.ts` が同時要求をまとめ、読み込みと反映を直列に実行します。読み込み中に届いた更新は次のバッチで確認します。`get_frontend_cache_batch` は変更のない行をカバリングインデックスから確認し、その JSON を読み込まずにメタデータだけ返します。変更した行のメタデータと JSON は同じ SQLite 読み取りトランザクションで取得します。`get_frontend_cache_batch`、`get_backend_task_timestamps`、`get_schedule_snapshot` は読み取り・時間割の構築・応答の JSON 化・読み取った DTO の破棄まで blocking pool で実行します。`background_ipc` の raw JSON 応答を使い、JSON string を外側へ追加することなく既存の object 応答を保持します。

MockRuntime の実際の IPC テストでは、旧方式の型付き応答と三命令の完全な JSON、省略フィールド、引数エラー、返される DB エラーを比較します。読み取り helper は共通なので、この比較は DB の意味論を独立した旧実装と比較するものではありません。別の worker テストは大きな cache 本文の応答を使い、JSON 化を一度だけ背景スレッドで実行し、DTO もそこで破棄することと、その処理中も単一スレッドの async executor が進むことを確認します。生成した `tests/fixtures/cache-batch-reply-wire.json` をフロントエンドで再生し、通知の同一性・版・更新通知と timestamp の秒からミリ秒への変換を検証します。実際の WebKit、GPU、アプリ全体の RSS・実行時間はこの検証に含めません。

スナップショット状態と AI 時間割キャッシュの読み取りは、`QueryReturnedNoRows` だけを未保存の `None` とします。テーブル欠落、列の型違いなどの SQLite エラーを未保存として扱い、空の時間割で表示を置き換えることはしません。保存済み selector・community JSON の配列への互換 fallback は維持します。ネットワーク同期も旧状態を先に読むため、その fallback を一律に拒否すると、同期で修復可能な旧 JSON まで更新を開始できなくなります。AI cache JSON の既存の解析エラーは引き続き返します。

`get_snapshot_state` と `get_ai_schedule_cache` は SQLite から所有する文字列を取得した時点で DB Mutex を解放し、その後に JSON を解析します。対応する保存処理も JSON 化を済ませてから DB Mutex を取得します。更新時刻は書き込み用のロックを取得した後に決めます。一方、複数テーブルを読む `build_raw_data` の持つ一連の読み取り範囲は維持します。

時間割の構築では、AI の週ラベル検証に最初に読んだ `SnapshotState` を渡します。AI 検証のために同じ行をもう一度 SELECT・解析せず、応答途中の別の同期で変わった週ラベルを混ぜません。未保存の `None` と保存済みの空 state は AI の扱いが異なるため、その区別も渡します。単独の AI cache 読み取りは従来どおり自身で metadata を取得します。テスト用の SQLite trace が実際の構築経路の SELECT 回数を確認し、旧実装では二回だった読み取りが一回になることを検証します。凍結した旧 AI loader と、未保存・空・有効な metadata、週の一致・不一致、空の AI 配列、期限内・期限切れの六十三組の完全 JSON を比較します。途中で metadata を更新した場合と解析・SQL エラーも別に確認します。この回数と持つロックの範囲は、アプリ全体の速度・RSS・GPU の改善率を示す測定ではありません。

`build_raw_data` は授業計画・KG-Course 詳細・Luna 件数・活動の本文を全件取得してから捨てる処理を省きます。KGC のキーは正常に読み取れた表示週の授業から選ぶので、壊れた授業行だけに属する計画を追加しません。Luna は covering index から ID のみを読み、既存の Rust の byte offset・年度・学期の判定を使ってから本文を取得します。未知の ID、Unicode、sentinel 年度、通年コース、空の学期の fallback を SQL の文字単位の `substr` で置き換えません。ID の metadata 読み取りは履歴の異なる ID 数に比例しますが、非表示の本文は読み取り・JSON 解析・DTO 構築をしません。

`db/scoped_rows.rs` はキーをパラメータに bind して二百五十六個ずつ読みます。既存の索引を使い、DB migration は不要です。授業計画のセッション順、件数・詳細の従来の rowid 順、活動の ID・種類・同種内の順序を保持します。旧 HashMap の計画グループ順は未規定のままです。表示キーが空でも列を含めた query を prepare し、テーブルや列の欠落を空の成功にしません。SQLite のエラー種別と原因を維持しますが、診断に含まれる SQL と文字位置は新しい query に対応します。行ごとの壊れた本文・JSON の既存 fallback と、一連の処理中の DB Mutex の範囲は保持します。

六項目のテストで、凍結した旧装配処理と本文全体を比較します。未規定の計画グループだけを並べ替え、それ以外の配列順は変えずに比較します。週の交換・重複・空白、Unicode と引用を含むキー、未知の Luna ID、空の学期、五百以上のキーの分割、壊れた行・JSON、空キー時の schema error を含みます。SQLite PROFILE の `FULLSCAN_STEP` は四種類の本文 query の全表走査がなくなることを確認し、ID metadata は query plan の covering index を確認します。旧装配処理は以前の全件 query と共通の row decoder を使うため、比較の対象はキー選択・装配と返されるデータです。

合成測定は `SELAH_RAW_SCOPE_BENCHMARK=/tmp/selah-raw-scope-benchmark.json cargo test --manifest-path src-tauri/Cargo.toml --lib db::schedule::raw_tests::benchmark_raw_scope_with_historical_bodies -- --exact --ignored --nocapture` で実行します。debug の一時 DB、表示中 KGC 二科目、四回の warmup と順序を交互にした二十一回の中央値で、履歴零科目は 0.446→0.454 ms、三十二科目は 5.11→0.49 ms、二百五十六科目は 35.72→0.73 ms でした。これはこの端末・fixture の読み取り・解析・選択・装配のみの値で、外側の応答 JSON 化、返す DTO の破棄、実際のアプリの RSS・WebKit・GPU は対象外です。

一時 DB のテストは未保存・正常データの往復、欠落テーブル、列の型違い、既存 JSON fallback とデータを変更しない読み取りを確認します。原生 IPC のエラー fixture をフロントエンドで再生し、表示中の時間割・版・更新通知を失敗時に保持し、次の成功応答を反映できることも検証します。fixture の再生成はリポジトリルートで `SELAH_CACHE_BATCH_REPLY_WIRE="$PWD/tests/fixtures/cache-batch-reply-wire.json" SELAH_SCHEDULE_READ_ERROR_WIRE="$PWD/tests/fixtures/schedule-read-error-wire.json" cargo test --manifest-path src-tauri/Cargo.toml --lib frontend_cache::ipc_tests` を実行します。

キャッシュ状態は `cacheStore.ts` に分離し、既存の `stores.ts` から再エクスポートします。`cachedBackendFetch` のコールド読み込みと手動更新も同じ同期キューを使います。同じキーの同時読み込みは一つの処理と解析結果を共有します。localStorage のプレビューは即時表示用であり、タイムスタンプが新しくても SQLite で一度検証します。IPC が失敗した場合はオフライン表示を保持します。

キャッシュ無効化はキーごとの世代を進めます。キュー登録時と応答の反映時に世代を確認し、ログアウトなどでクリアしたデータが古い応答から復活することを防ぎます。処理終了時は自分の Promise だけを待機表から除去します。通常の `cachedFetch` と `refreshCache` もこの規則を使い、失敗後の再取得は待機していた呼び出し間で共有します。自動更新中の手動強制更新は、一度だけ強制の後続処理を実行します。

時間割の版は、コース・授業計画・件数・活動・詳細・AI 結果・スナップショット状態・KG-Course の警告を追跡します。派生データには日付、春秋の学期開始設定、AI 結果の有効期限も影響するため、バックエンドがこれらを含む不透明なスタンプを返します。フロントエンドはスタンプを解釈せず保存します。生成 TODO の合成には元の Luna 配列を使い、削除された生成項目が画面の合成配列から復活することを防ぎます。

### 汎用 cache IPC の受付と背景処理

`get_data_cache`、`get_data_cache_updated_at`、`save_data_cache` は `app_state/cache.rs` で受け付けます。同期 command で DB の Mutex・SQLite を待ち、その場で大きな本文を JSON 化する経路を、既存の `background_queue.rs` の専用インスタンスへ移します。応答の future を poll する前に受付を確定し、同じアプリの複数 WebView から来たこの三命令を受付順に実行します。後の cache 読み取りが先の cache 書き込みを追い越さず、応答を捨てても受理済みの保存は完了します。別の命令や backend の内部 DB 操作まで全体を直列化する保証ではありません。

引数は worker 内で Tauri の `CommandItem` から借用します。`key` と `json` の大きな文字列をもう一つ所有するための複製を省き、型違い・必須引数の欠落・bytes payload の既存エラーを保持します。読み取りの JSON string または null、timestamp の scalar または null、保存の null 応答は `background_ipc` の raw `Response` として返します。完全な cache 本文、空文字列、非 JSON の保存本文、`seen_notifs_` の保存禁止、読み取り時の DB エラーを null にする挙動、保存時の DB エラーを拒否する挙動も保持します。

五項目の Rust テストは一時 DB と Tauri の MockRuntime の実際の IPC 経路を使います。凍結した旧同期命令と、数 MiB の日本語・Unicode・改行・引用・null 文字を含む全文、null・零・負の timestamp、不正な UTF-8 cache、引数エラーと DB エラーを比較します。入力本文の pointer 一致で借用を確認し、worker を止めた状態でも二つの WebView から受付と別命令が進むこと、三十二回の交互の保存・読み取り、逆順の応答待機、応答の破棄、終了境界・drain・受付再開を検証します。実際の macOS WebKit、IPC 本文を受信時に解析する処理、GPU、アプリ全体の速度・RSS はこの検証に含めません。

### 通知の既読状態と cache の順序

`mark_notification_read`、`mark_batch_notification_read`、`get_read_notifications` も同じ cache queue に入れます。三つの旧同期 command が行っていた DB 待機、既読 JSON の解析・構築、旧ファイルの移行を worker 内で実行します。汎用 `save_data_cache` で `read_state` を書く処理との順序も保持し、複数 WebView の既読更新が read-modify-write の途中へ割り込みません。batch の ID 文字列は受信した JSON 値から借用し、有効な新しい ID だけを既読集合のために所有します。空・五百十二 UTF-8 byte 超・既知の ID の除外、三 source ごとの五百件の上限、既存の null 応答と引数エラーは維持します。上限の除去対象は従来の HashSet の選択で、時系列による保持ではありません。

`stores.ts` の公開既読操作も同じ WebView 内で受付順に IPC・store 反映を終えます。後の標記が初回読み取りの応答を追い越して、その古い一覧で新しい既読 ID を消さないようにします。同時の読み取りは一つの Promise を共有し、標記を受付けた後の読み取りはその標記の後に新しく取得します。失敗は現在の呼び出しへ返し、後の操作を止めません。batch の配列は受付時に浅く複製し、その後の呼び出し元の配列変更を反映せず、同じ batch の重複 ID も store に一回だけ追加します。

デモの読み取りは旧世代を無効にして store を即時初期化します。旧応答・旧失敗はデモの既読を変更せず、まだ発行していない旧操作は IPC を開始しません。デモ内の標記は実 DB の遅い応答を待たずにローカル反映します。既に発行した native 書き込みを取り消す処理ではありません。旧 `read_items.json` の移行では、DB への保存に成功した場合だけ元ファイルを除去します。保存が失敗すれば元の byte を保持し、DB が未移行なら次の読み取りで再試行できます。

通知 IPC の五項目は Tauri の MockRuntime と一時 DB で旧 wrapper の引数・null 応答・完全な既読集合、無引数 command の bytes payload、二つの WebView と汎用 cache の交互操作、応答の破棄・終了境界・受付再開を確認します。配列の順序は既存の HashSet 同様に不定のため、集合を比較します。`read_state/tests.rs` の七項目は五百十二 byte の境界、重複・不正 ID、三 source の分離と上限、監査 trigger による不要な DB UPDATE の非発生、移行の DB 書き込み失敗・元ファイル保持・成功後の再接続、不正な旧ファイルの保持を検証します。テストはユーザーの旧ファイルを読みません。

`tests/read-state.test.mjs` の十五項目は製品の store 処理と Svelte の writable を代替 IPC・localStorage 境界で実行します。百読み取りの一回への合流、三十二回の逐次標記、読み取り・標記・次の読み取りの順序、失敗後の進行、配列変更と重複、古い Promise の cleanup、store callback からの再入、デモへの切替前の読み取り・標記の遅い成功と失敗、localStorage がない場合を確認します。実 WebView の速度・CPU・RSS・GPU や LIVE 白画面への効果を測定したものではありません。

### 既読の保存失敗と不正なデータの保持

既読状態の DB 読み取り・JSON 解析・保存、旧ファイルの読み取り・解析・移行保存は `Result` を返し、cache queue の IPC 拒否までエラーを渡します。DB 保存を捨てて null を返していた経路を除き、現在の前端 caller も失敗を受け取ります。store は保存成功後だけ標記を反映し、古い読み取りの失敗も現在の一覧を空配列に置き換えません。DB 行と旧ファイルの両方が存在しない場合だけ新規の空状態を作ります。読めない DB がある場合に旧ファイルへ戻したり、不正な既読 JSON を空状態として新しい ID だけで保存したりしません。元の本文・file は保持し、保存や内容を復旧した後の明示的な次の操作は通常の queue で進みます。

無効な単一 ID と不明 source は既存どおり成功の無操作として返し、不要な読み取り・移行を開始しません。source ごとの上限と既知 ID の書き込み省略は維持します。読み取り応答の ID は parse 済みの HashSet から Vec へ所有権を移し、すべての文字列を返す際にもう一度複製しません。

追加の原生五項目は一時 DB の保存拒否 trigger、壊れた JSON・必須 field の欠落・不正 UTF-8、テーブルの読み取り失敗、旧ファイルの読み取り失敗と未存在を検証します。失敗後の全文・他 source の ID の保持、既知 ID の無書き込み、再試行、旧 DB-backed 経路の偽の成功と不正 cache の置換も比較します。以前の移行検証も、保存失敗が caller のエラーになることを含めます。試験用の旧経路は file migration を呼ばず、ユーザーの旧ファイルを参照しません。

`tests/fixtures/read-state-error-wire.json` は MockRuntime の実際の IPC から取得した保存拒否・解析失敗・成功再試行の返答です。原生テストは現在の返答とこの fixture を比較し、前端の追加二項目は同じ拒否文字列・null 成功・完全な ID 集合を製品の store 処理に渡します。単一・batch の拒否で store が変化せず、読み取り拒否で一覧を保持し、後の再試行が反映されることを確認します。契約変更時の明示的な再生成には `SELAH_READ_STATE_ERROR_WIRE="$PWD/tests/fixtures/read-state-error-wire.json" cargo test --offline --manifest-path src-tauri/Cargo.toml --lib app_state::cache::tests::persistence_errors_reach_ipc_and_retries_preserve_every_source` をプロジェクトルートで実行します。これは実際の WebKit・DOM・ユーザー DB・GPU を使う検証ではありません。

## LIVE の状態と描画

STT の確定行は Rust の LIVE セッションへ追加してから `live-transcript-appended` で配信します。イベントはセッション ID・行番号・追加行だけを含み、過去の字幕やホワイトボード全体を送りません。フロントエンドはイベント欠落時や再マウント時にスナップショットを読み、別セッションのイベントと古い字幕スナップショットを区別します。

`live-session-updated` は活動・講義・開始時刻・要約と終了の状態、および字幕・未要約行・要約の件数を送ります。新しい要約が追加された場合だけ最後の一段を付け、過去の字幕・未要約行の配列・全要約の配列を繰り返し送りません。最新要約の参照を Arc で保持し、セッションロックを解放してからその一段を JSON 化します。字幕の参照を通知に保持せず、要約・ホワイトボードをロック内で深く複製しません。定時要約の開始も小さい状態通知で知らせます。

全体状態・開始時の復元・日別キャッシュのプレビュー・手動字幕の応答・要約要求・全体要約の文字列・終了保存の応答は `live/response.rs` と共通の `background_ipc.rs` で JSON 化します。セッションと保存のロックを解放してから blocking worker でエンコードし、`tauri::ipc::Response` の JSON 本文として返します。Tauri の async 継続で全履歴を再度シリアライズしません。完全な記録用 API は字幕・待機行・全要約・ホワイトボード・Markdown と既存フィールドを保持し、ページ用 API は下記の表示用応答を使います。JSON オブジェクトを文字列へ二重変換せず、全体要約は従来どおり JSON 文字列として復号され、Markdown 全文とエスケープを保持します。ページ用の保存済みイベント `live-surface-saved` とページ用の終了応答、完全記録用の `live-session-saved` と完全記録の終了応答は、それぞれ同じ一回のエンコードを共有します。イベントには public な `Emitter::emit_str` で JSON 本文を渡し、二つの通信先の文字列バッファはそれぞれ保持します。

手動字幕の `live_append_transcript` は `live/admission.rs` が同期で検証・追加し、その時点のスナップショットを保持してから背景の応答処理へ渡します。応答の最初の poll が遅れたり待機元が破棄されたりしても、受理済み字幕は失われません。その後に別の録音が始まっても旧応答の内容を新しい録音から読み直しません。その他の命令は Agent の受付 handler と generated handler に委譲します。命令名と引数形式は保持します。

共通 worker の回帰テストは単一 async スレッド上で、処理・シリアライズ・大きな応答データの破棄が別スレッドにあること、エンコード待機中も別の async task が動くこと、保存イベントと応答のエンコード回数が一回であることを確認します。LIVE の実際の状態読み取りで 10,000 行と要約全文を比較し、エンコード中の追加・録音置換、両ロックの解放、手動字幕の poll 前の受理、破棄された応答、非活動・空保存と元のエラーを検証します。実機 UI や WebKit GPU 障害を再現するテストではありません。

全体読み取り・開始応答・終了結果・状態通知の取得には、同じセッションロック内でプロセス共通の `update_revision` を割り当てます。順序は録音の終了と置き換えを跨ぎ、古い終了通知や遅い全体読み取りで新しい録音を消したり復活させたりしません。フロントエンドの順序記録は授業履歴のプレビューに切り替えても保持します。古い同一録音の全体応答は欠けた字幕や要約を補えますが、新しい状態・終了段階・要約済みの行範囲を戻しません。

受信した要約が次の一段なら、その参照だけを既存配列へ追加します。既存の字幕・要約・ホワイトボードは参照を保持します。未要約行は「通知時の全行数 − 未要約行数」を消費済みの先頭部分として計算し、AI 生成中に届いた新しい字幕を残します。字幕や要約の件数に欠落があるときだけ、`createCacheSyncQueue` でまとめた復旧読み取りを要求します。読み取り中の新しい要求は次の一回にまとめ、前の読み取りが失敗しても後続を実行します。最新の一段だけを欠けた先行要約の位置へ挿入しません。非活動の通知はアクティブな録音を解除しますが、終了済み記録やプレビューを受動的に消しません。

共通の `createCacheSyncQueue` は、未開始の一つの batch に key の集合・強制更新条件・完了 Promise を一つずつ持ちます。同じ batch の要求は同じ Promise を返し、通知ごとの内部 waiter 配列や Promise を作りません。実行開始時に pending 枠を解放するため、読み取り中や同期 apply 中の再入要求は別の一 batch になります。key の初出順・強制更新の優先・直列実行・batch ごとの失敗を保持し、空の要求は参加も強制更新も行いません。LIVE の `resyncSession` は読み取り失敗をその batch 内で一度記録し、完了を直接共有します。従来どおり公開呼び出しは失敗を外へ投げず、その間に到着した復旧要求も次に実行します。

共通 queue の九テストと実際の LIVE recovery block の五テストは、二回の一万通知、完了の共有と batch 間の分離、同期再入、空要求、原始値の例外、失敗後の後続、録音置換と破棄後の IO 省略を確認します。保存した直前の実装では Promise 同一性の新しい検証だけが失敗します。LIVE の比較は共通 queue を現在版のまま固定し、page で追加していた catch の影響を分けます。`node scripts/benchmark-cache-sync-queue.mjs` は直前の queue と現在版で二つの同期通知 burst の enqueue 処理を比較し、key・強制更新・読み取り回数の一致と返した完了 Promise の個数を確認します。この環境の九回交互計測（三回 warmup）の中央値は、各 200 要求で合計 0.064 → 0.022 ms、各一万要求で 2.097 → 0.429 ms、返した異なる完了は各 burst の要求数 → 一個でした。Node の合成試験であり、drain 時間・呼び出し側の Promise reaction・IPC・読んだデータ・DOM・アプリの RSS/GPU は含みません。Promise の同一性はヒープ割当量の測定ではありません。

macOS・Windows の字幕浮窗は `LiveSessionStatus` として順序と活動状態だけを読み、要約 JSON の値を動的な配列や辞書へ展開しません。古い状態通知を除外し、閉じている間も順序を追跡します。通知の変更はネイティブとフロントエンドを同じビルドで更新する必要があります。

`live-session-updates.test.mjs` は通知の重複・前後逆転、要約中の新しい字幕、欠落の復旧、開始応答より先の通知、終了結果の表示と古い録音の拒否を検証します。Rust テストは実際の JSON シリアライズで過去の配列が含まれないこと、最新の一段の不変性、全体読み取りと通知の取得順序を確認します。10,000 行と 1,001 段の合成記録では、全体スナップショット 4,622,254 バイトに対して状態通知は 401 バイト、最新要約付きは 516 バイトでした。最新要約には実際の本文とホワイトボードを含むため、その一段の成長に応じて通信量も増えます。この比較は合成 JSON 本文だけであり、実機の CPU/RSS や GPU 障害の改善率ではありません。

### トレイのメタデータ読み取りと寿命

`live_get_tray_status` は活動状態・実際の録音状態・開始日時だけを返します。LIVE → STT の既存のロック順序で、LIVE の ID を保持したままマイクの所有者と段階を確認します。非活動時は STT を読まず、転写履歴・要約・講義情報・ホワイトボード・モデル設定・ファイルを取得しません。トレイの復旧確認は、以前の全 LIVE スナップショットと別の STT 状態の二回の IPC を、この一回に置き換えます。実際の JSON シリアライズを使う回帰テストでは、転写履歴と未要約行に同じ 10,000 行を持つセッションの全スナップショットは 2,038,137 バイト、トレイ応答は 67 バイトで、履歴を追加してもトレイ応答は変わりません。値は通信の包みを含まない本文のサイズであり、実機の CPU/RSS や GPU 障害の改善率ではありません。

トレイの取得と JSON 変換は `live/commands/tray_status.rs` の blocking worker に置きます。IPC の同期 handler で LIVE/STT の mutex を待たず、async executor にもその待機を置きません。返答は従来と同じ三フィールドの JSON オブジェクトで、原文 JSON を文字列値として二重変換しません。取得は worker が lock を得た時点の現在状態を使います。ロック順序と同じ録音 ID の照合、非活動時の STT の省略、エラーを非活動状態へ偽装しない規則を保持します。

`stt/ipc_status.rs` は `stt_is_running`、`stt_get_active_caller`、`stt_get_stream_state` の IPC wrapper だけを非同期にし、同じ共通 worker で既存の同期 domain 関数を読み、JSON を変換します。登録先をこの wrapper へ変更し、JavaScript の command 名と bool・文字列/null・状態 object の形式は保持します。macOS/Windows の native shortcut とトレイ内の coherent read は従来の同期 domain 関数を使います。状態照会にモデル・設定・履歴の読み取りは追加しません。

トレイ worker の五テストは一万行の履歴参照数と revision が変わらないこと、全 metadata の JSON 一致、単一 async スレッドでの LIVE lock と代替 STT read の待機、エラー、実際の MockRuntime handler が busy LIVE lock を待つ前に IPC 呼び出し元へ戻ることを確認します。旧同期 wrapper と現在の四 command を実際の IPC に通し、object・bool・null と失敗文字列を比較します。STT worker の二テストは全 phase と完全な Unicode 所有者・ID の JSON byte 一致、代替状態 mutex の待機と poison のエラーを確認します。これらは thread と wire の検証で、マイク・モデル・OS の描画や実機の GPU/RSS の測定ではありません。

`tests/fixtures/status-read-wire.json` は現在の実際の四 command の MockRuntime IPC、代替 LIVE 録音と poisoned LIVE state の応答です。STT はアプリを起動せずに既定の inactive state を読みます。プロジェクトルートで `SELAH_STATUS_READ_WIRE="$PWD/tests/fixtures/status-read-wire.json" cargo test --manifest-path src-tauri/Cargo.toml --lib live::commands::tray_status::tests::actual_ipc_status_commands_match_legacy_objects_scalars_nulls_and_errors` を実行して再生成できます。フロントの実際のトレイ制御と STT API はこの fixture の object と文字列エラーを再生し、pause 表示、失敗時の直前の正常表示、非活動への復旧と一回の STT 状態 IPC を確認します。フロントでの通信境界は代替 invoke です。

トレイの開始ごとに `ResourceScope` を作り、キャッシュ・ネイティブ購読、登録待機中の解除ハンドル、300 ms のまとめ待ち、90 秒の復旧タイマーを所有します。停止した世代の遅い登録・コールバック・読み取り・タスク状態更新を次の開始で採用しません。時間割は管理済みの共有キャッシュを使い、欠落時だけバックエンドから読み取ります。その復旧結果は新しいキャッシュ通知・停止で無効にします。デモの時間割を実際のバックエンドで置き換えません。

`TrayStatusRefresh` は通知で取得中の結果を直ちに無効にし、`createCacheSyncQueue` で同時要求を一回にまとめます。読み取り中の通知にも次の取得を行い、重複した取得を並行させません。失敗時は最後の正常なトレイ表示を保持します。`TrayStatusWriter` は停止時の空表示を含む書き込みを開始世代を跨いで順に送り、未送信の古い要求を省きます。同じ表示項目を再送してネイティブの輪播位置をリセットせず、失敗した書き込みは正常な値として記憶しません。

`tray-status.test.mjs` は実際のモジュール・共有キャッシュ・Tauri 通信ラッパーを代替通信で動かし、メタデータだけの IPC、同じ表示の省略、再開後の遅い購読、旧世代の結果、新しいキャッシュの優先を検証します。`tray-status-refresh.test.mjs` は通知集中、取得中の更新と即時無効化、読み取り失敗、停止と再開を跨ぐ書き込み順序、失敗後の再試行を検証します。

### 音声入力の停止と所有権

停止要求と停止完了を `SttSessionControl` で分けます。通常の停止ではマイクを閉じ、受信済み音声を VAD に渡し、末尾を確定キューへ入れてから認識器のスレッドを join します。未処理の仮字幕だけを除去し、確定行は順番どおり処理します。モデルとマイクの解放、`idle` 通知が済むまで入力の所有権を保持します。次の入力開始や入力の借用は前の処理完了を待つため、終了中のモデルに重ねて別のモデルを起動しません。

Tauri の開始・停止待機は blocking pool で実行します。ネイティブショートカットは待機せず要求だけを返し、最初の確定行や時間切れではなく `idle` を受けて全区間をまとめて送信します。Agent の入力欄も各確定行を追加し、末尾の一行で前の発言を上書きしません。

`stt_get_stream_state` はマイクの予約ロック一回で段階・マイク ID・使用者・LIVE 録音 ID を返します。モデル初期化中と録音中を区別し、停止要求後は確定処理中も `stopping` と所有者を返します。モデル設定・ファイル・デバッグ情報は読みません。LIVE と二つの Agent 画面はこの読み取りを使い、独立した running/caller の二回の IPC を除去します。トレイは後述のメタデータ専用読み取りの中で同じ STT 状態を参照します。LIVE は録音 ID の一致も確認し、フォーカス時の再読み取りで無音時間の起点をリセットしません。Agent の読み取りも `LatestViewRead` で通知・新しい読み取り・終了後の古い結果を抑止し、一時的な IPC エラーだけで録音停止を表示しません。

Agent の開始・停止 IPC には `inputSessionId` が必要です。`caller` は機能を表し、入力 ID はその画面が開始するたびに生成する UUID です。STT の予約、状態スナップショット、字幕・状態・情報・エラー通知に同じ ID を付けます。停止要求は caller と入力 ID を同じ予約ロック内で照合し、閉じた Agent A の停止要求が新しい Agent B を止めません。LIVE の録音 ID と、内部のマイク番号とは別の識別子です。macOS と Windows のネイティブショートカットも各開始の UUID を使い、入力 ID なしの Agent 開始・停止を拒否します。終了時の全入力停止は引き続き内部の全所有者停止を使います。

`AgentSpeechInput` は二つの Agent 画面の入力寿命と借用復帰を共通管理します。開始要求前に UUID を設定するため、開始 RPC より先に届く初期化・字幕・終了を扱えます。他の画面や旧入力の字幕・状態・エラーは表示せず、停止完了待機中の自分の末尾の確定行は受け付けます。ページ終了で自分の ID の停止だけを要求し、開始が待機中だった場合は開始応答後にも同じ ID の停止を確認します。その後に原使用者を復帰させ、閉じた画面は更新しません。入力を所有していない初期画面では状態 IPC も送りません。

`SttInputState` は実マイクの予約と原使用者を一つのロックで管理します。Agent が LIVE を借りるときは、その録音 ID を返します。Agent 間の切り替えは原使用者を次の入力へ引き継ぎ、停止・解放後から次の予約までの空白期間にも保持します。旧 Agent 自体を再起動しません。帰還は借用を許さない開始を使うため、別の入力が動作中なら拒否し、LIVE が終了・保存中でも ID の検証で拒否します。帰還や別の非 Agent 入力の開始が成功すると、保留していた原使用者を消費します。初期化エラーで `idle` が応答より先に来ても、応答後に一度だけ復帰を試みます。

`agent-speech-input.test.mjs` は実際の共通制御を代替通信で動かし、登録前の入力 ID、遅延した開始応答とページ終了、早い `idle`、旧入力の通知と読み取り、重複開始、末尾の受付、開始失敗後の再試行を検証します。Rust では停止の ID 照合、Agent 間の原 LIVE 引き継ぎ、解放後の空白期間、二重予約の拒否と通知の ID を検証します。実機のマイクや GPU 障害を再現するテストではありません。ネイティブ命令とフロントエンドは同じビルドで更新します。

設定の初回読み取り・保存、モデルのファイル状態確認・削除・テストも非同期 IPC から blocking pool に送ります。モデルテストは一回の設定スナップショットでモデルと実行バックエンドを選びます。テスト・ダウンロード・削除は共通のモデル操作予約を取り、録音開始と同じ入場ロック内で録音が存在しないことを確認します。停止要求済みでも、確定処理中の録音がモデルを保持している間は操作を拒否します。操作のために録音を停止しません。

モデル操作中は予約だけを保持し、録音開始用のロックを長時間占有しません。その間の新規入力や重複したモデル操作はすぐにエラーを返し、長いダウンロードの終了待ちになりません。ネイティブのモデル解放とファイル処理が終わってから予約を解放し、失敗や Rust の panic でも解放します。IPC の待機元がなくなっても実行中の blocking 処理は継続するため、予約は実際の終了まで保持します。設定保存と字幕頻度の変更はこのモデル予約とは独立し、録音中も利用できます。

モデルの再ダウンロードは既存のモデルを先に削除せず、同じファイルシステムの専用ステージングディレクトリに受信・展開します。既知の HTTP 本文長、非空の通常ファイル、モデルの最低サイズ、tokens と必要な VAD の準備を確認してから正式な保存先へ移します。アーカイブのリンクや想定外のディレクトリは展開しません。転送と展開は分割読み取りの間で中止を確認し、失敗・中止時は準備中のファイルだけを除去します。ネットワーク読み取り自体は既存の blocking HTTP 呼び出しであり、中止が確認されるまでその読み取りを待つ場合があります。

配置では既存のディレクトリ・VAD を一時退避してから rename し、途中の IO 失敗は逆順の移動で復元します。復元にも失敗した場合は旧ファイルを含む退避先を消さず、エラーに場所を示します。配置中の Rust の panic でも退避先を保持し、ログに場所を記録します。モデル操作予約はこの処理と清掃の終了まで保持します。100% の完了通知は配置後に送り、転送中の MB 表示には実際の受信バイト数を使います。圧縮アーカイブは成功後も永続キャッシュせず、古いビルドの圧縮ファイルは成功時だけ除去します。この配置は複数の rename を使うため、rename 間のプロセス終了や停電を跨ぐ完全なトランザクションではありません。

インストールの回帰テストは一時ディレクトリと小さな実際の bzip2/tar を使い、正常な置換、不完全な転送、破損・不完全なアーカイブ、展開中の中止、配置失敗時の復元、復元失敗時の退避保持を確認します。ユーザーのモデルを削除したり、ネットワークから大きなモデルを取得したりするテストではありません。

`live_finish_session` はバックエンドでも停止完了を確認してから保存します。待機が 30 秒を超えた場合は終了中のセッションを保持してエラーを返し、再試行できるようにします。停止要求には呼び出し元を指定し、LIVE の停止が借用中の Agent マイクを止めることを防ぎます。LIVE の開始要求・字幕・状態・エラーには録音セッション ID を付け、キャンセル済みの録音から新しい録音へ発言や状態が流れ込まないようにします。アプリ終了の中断フラグはプロセス終了まで保持します。

回帰テストでは遅延したデコードを模したスレッドで実際のキューと worker の終了処理を検証し、全確定行の処理前に停止完了しないこと、入力の所有者と録音 ID の隔離を確認します。独立した入場ゲートと代替モデルを使い、操作中の開始・重複操作・確定処理中の操作拒否、失敗時の解放順序も確認します。単一スレッドの async runtime と遅延した blocking 処理で、待機元のキャンセルがモデル予約を早く解放しないことを検証します。実際のマイク、ONNX 推論、WebKit GPU 障害の再現は別途実機検証が必要です。

### 音声の前処理とサンプル時計

`audio/onset.rs` はノイズゲートで除外した直近 200 ms（16 kHz mono の 3,200 サンプル）を保持します。次のチャンクが従来の門限・発話中・再開猶予の条件を満たした時、保持した音声を一度だけ先頭へ戻してから AGC と VAD に渡します。徐々に大きくなる発話の弱い始まりを、音量の門限だけで直ちに失わないための処理です。門限や VAD の感度は変更せず、静音中の保持だけでは VAD 推論を実行しません。保持用の配列は最初の静音で 3,200 サンプル分を確保し、その後は約 12.5 KiB の同じ容量を使います。長い入力チャンクでも末尾 200 ms だけを保持し、音声の再投入時は現在のチャンクにその前置部分を加えます。全発話が門限未満のままで終了した場合は、この保持だけで新たな発話とは扱いません。

回帰テストは弱い始まりの順序と一回だけの再投入、長い室内音の容量上限、大きなチャンク、コールバックの分割、空入力、次の録音への非継承を確認します。公開音声を使う追加の CPU VAD テストでは、保持する末尾 200 ms を独立して組み立てた基準と、実際の VAD に渡す全音声・確定区間を比較し、`VadInputTail` のフレーム余りも合わせて確認します。これは音声の保持と接続の検証で、弱い語音の認識や全体の文字誤り率が改善したことを測るテストではありません。

`session/vad_tail.rs` は VAD に実際に渡した音声のフレーム余りを保持します。sherpa-onnx 1.12.39 の `Flush` は未処理の入力バッファを処理しないため、マイクとリサンプラーの受信済み末尾を渡した後、最後の 512 サンプルのフレームだけをゼロで完成させてから確定します。追加は最大 511 サンプル（16 kHz で約 31.94 ms）で、空入力やフレーム境界では追加しません。VAD に渡さずノイズゲートで除外した音声は数えません。録音の取消や認識 worker の失敗時にはこの確定処理を行わず、完了した末尾も一度だけ配信します。仮字幕の最終窓を再び確定する処理ではありません。

通常のテストは全ての非空の余り、空・境界、異なるコールバック分割を確認します。追加の `real_vad_retains_the_incomplete_speech_frame_on_stop` は CPU の実際の Silero VAD と公開テスト音声を使用し、1・127・256・511 サンプルの末尾で独立してフレームを完成させた基準と全波形を比較します。旧フローでの末尾欠落、実際の末尾サンプルの保持、二重確定の防止、静音から語音区間を作らないことを確認します。このテストはローカルのモデルと 16 kHz mono の little-endian f32 音声を必要とするため通常実行では除外します。`SELAH_STT_VAD_MODEL` と `SELAH_STT_VAD_SAMPLES` を指定して `cargo test --manifest-path src-tauri/Cargo.toml --lib stt::session::vad_tail::tests::real_vad_retains_the_incomplete_speech_frame_on_stop -- --exact --ignored` で実行できます。マイクや SenseVoice 認識器は起動せず、単語の認識精度や GPU 障害を測るテストではありません。

確定字幕の配信は一つの認識 worker が持つ `FinalTranscriptGate` で取得順序番号を確認します。同じ番号や古い番号の再配信を拒否し、別の VAD 区間で同じ文が認識された場合は両方を保持します。従来の本文一致だけの除外では「はい」「Yes」などの連続発話まで失われていました。確定ジョブは取得時に固有の番号を予約し、確定用キュー内の FIFO を保持します。空の認識結果は配信せず番号も消費しません。ゲートは録音ごとに新しく作り、仮字幕の順序判定とは独立します。配信ゲートの回帰テストは複数言語の反復、同じ番号で異なる本文を持つ再配信、古い番号、空結果と次の録音を確認します。フロントエンド側も別の行番号の反復を表示用配列と入力草稿に保持し、同じ字幕差分の再配信だけを無視することを確認します。音声モデルが別区間で同じ誤認識をした場合も本文だけでは除外できないため、実際の音声認識精度の測定とは区別します。

マイクの `session/input.rs` は CPAL 0.15 が公開する 10 種類のサンプル形式（符号付き・符号なし 8/16/32/64 bit、32/64 bit 浮動小数点）を受け付けます。従来は F32/I16/U16 だけで、他の形式は録音開始前にエラーになっていました。F32 は従来の直接コピーを保持し、他の形式は CPAL の `FromSample` で F32 に変換してから既存のチャンネル混合とリサンプリングへ渡します。16 bit 整数を最大値で割る旧処理では、符号付き最小値が −1 を少し超え、符号なしの静音中点が非ゼロになっていました。新しい変換は静音中点と半振幅を正しく保持します。幅の大きい整数では F32 の表現精度による丸めは残ります。

入力変換の回帰テストは旧処理の二つの偏差、全整数幅の静音・半振幅・最大振幅、浮動小数点の振幅、空入力、44.1 kHz ステレオを不規則なコールバックで分割したときの波形一致を確認します。全形式の型付きストリーム構築経路も macOS コンパイルで確認します。実際のマイクは起動せず、特定のデバイスの動作確認や認識文字誤り率の改善を示すものではありません。

`audio.rs` の AGC は発話中の履歴ピークを基準に、最大 2 倍まで小声を増幅します。小声から大きな声へ変わった際に履歴が追いつくまで波形を削らないよう、現在の音声チャンクのピークでも増幅率を制限します。VAD が発話を検出する前のチャンクにもこの制限を適用し、一つのチャンクには同じ増幅率を使います。静音は履歴ピークを更新せず、既に入力側でクリップした波形は復元しません。回帰テストでは旧処理で再現する二種類の波形の削れ、小声の増幅、最大振幅の入力、静音と空入力を確認します。これは波形の保持を確認するテストで、講義の専門用語に対する認識精度の測定ではありません。

現行の認識器は SenseVoice の INT8 モデルと `greedy_search` を使います。依存する sherpa-onnx 1.12.39 の SenseVoice 実装はこのデコード方式のみを受け付けるため、別方式の beam 設定や汎用 hotwords 設定だけで専門用語の認識を改善できるとは扱いません。仮字幕は直近 5 秒、確定字幕は VAD の区間を認識します。LIVE の要約生成には科目名などの文脈がありますが、元の認識本文を書き換える処理ではありません。モデルや用語文脈の変更を評価する際は、同じ日本語講義の音声と正解文で仮字幕・確定字幕を分け、文字誤り率、専門用語の正答、遅延とメモリを比較する必要があります。

`audio/resampler.rs` は入力フレームの位置を整数の位相で持ち、コールバック境界でも進め続けます。44.1 kHz などの入力を毎回丸めていた方式と異なり、分割サイズによる音声の長さや波形の変化を防ぎます。16 kHz 未満の入力も補間して 16 kHz に変換します。途中で分割されたステレオのフレームは次の入力へ引き継ぎ、マイク停止後は受信済み音声と最後の補間点を VAD に渡してから確定します。CPAL のバッファサイズは全チャンネルを含むフレーム数で指定し、約 80 ms の要求をチャンネル数で増やしません。

ダウンサンプリングでは従来の 63 タップ・7.5 kHz の FIR フィルタを保持し、出力に必要な位置だけで畳み込みを計算します。48 kHz 入力なら、出力される 3 フレームごとの位置だけを計算します。全入力フレーム分のフィルタ結果と中間のモノラル配列を保持しません。16 kHz のモノラル入力は入力を直接コピーする経路を使います。音声キューの確定区間を間引く最適化ではありません。

通常の Rust テストでは、複数の入力レート・不規則な分割・ステレオのフレーム境界について波形と出力長を比較します。48 kHz の旧波形との一致、低レートの補間、最後の補間点を一度だけ確定すること、1 kHz の通過と 12 kHz の抑制も確認します。比較用の旧実装はテスト内だけに置きます。

`scripts/benchmark-stt-resampler.sh` は実際の DSP モジュールを `rustc -O` で単独コンパイルし、信号テストと CPU ベンチマークを実行します。モデルや Tauri を起動せず、一時ファイルは終了時に削除します。2026-10-07 の macOS 測定はモノラルの 80 ms 入力を 1000 回処理し、ウォームアップ後、実行順序を交互にした 5 回の中央値を使いました。

| 入力レート | 旧処理の合計 | 現処理の合計 | 比率（旧 / 現） |
| --- | ---: | ---: | ---: |
| 16 kHz | 0.373 ms | 0.221 ms | 1.69 |
| 44.1 kHz | 141.587 ms | 88.560 ms | 1.60 |
| 48 kHz | 173.164 ms | 48.296 ms | 3.59 |
| 96 kHz | 310.875 ms | 47.014 ms | 6.61 |

この比較はリサンプリングだけのローカル測定です。ONNX 推論、アプリ全体の CPU/RSS、GPU 障害の改善率を示す値ではありません。

### 仮字幕モデルの寿命

LIVE の確定字幕と仮字幕は別の認識器・キューを使います。最省電 (`final_only`) で開始した場合は確定字幕の認識器だけを作成します。録音中に仮字幕を停止すると、設定変更の制御ジョブが待機中の仮字幕スレッドを起こし、その認識器を破棄します。再有効化時は同じスレッドで再作成するため、マイクの VAD 処理と確定字幕のキューを止めません。標準・省電モードでの開始は従来どおり仮字幕用の認識器を先に準備します。

設定キャッシュは保存成功時に仮字幕の有効状態と世代番号を同時に更新します。マイク側の設定確認の間に「停止→再有効化」が完了しても、前のモデルや字幕ジョブを区別できます。頻度だけの変更は世代を進めず、モデルを読み直しません。最初のモデル準備にもこの世代を付け、worker へ渡す間に古くなったモデルは音声の到着前に破棄します。初期ロード・再ロードの結果とエラー、デコードした字幕は停止要求と設定世代で判定し、過去の処理結果を通知しません。現在も有効なモデルの初期化エラーは従来どおり報告します。ネイティブのモデル初期化呼び出し自体は中断できないため、停止完了は実際の戻りと解放を待ちます。

設定の初回ディスク読み取りと保存は専用の IO ロックで直列化し、古い読み取りが保存後のキャッシュを上書きしません。保存失敗時は実行中の設定と世代を変えません。モデル・言語・実行バックエンド・VAD 感度は録音開始時に固定し、仮字幕の更新頻度だけを録音中に変更します。音声処理の一回の判定で同じ更新頻度プロファイルを使い、キューの有効状態と字幕処理の設定がずれることを避けます。更新頻度プロファイルは保存時に計算してキャッシュし、マイクの確認ループは文字列を複製せず、同じロック内からプロファイルと世代だけをコピーします。

モデル寿命の回帰テストはロード回数と破棄を追跡する代替モデルを使い、静音時の破棄、復帰時の一回だけのロード、確定字幕の継続、初期ロード・読み込み中・worker 受け渡し中の停止や設定変更を検証します。設定キャッシュのテストは独立したキャッシュと代替ライターを使い、短時間の切り替えと保存失敗、初回読み取りとの競合を確認します。これは実際の ONNX モデルの RSS や GPU の使用量を測った結果ではありません。

### LIVE の保存と復元

`LivePersistence` は保存要求を録音 ID ごとにまとめ、ディスク処理を blocking pool で直列化します。字幕追加は保存要求だけを登録します。保存する配列の参照と進捗を短いセッションロックで取り出し、JSON 変換・Markdown 構築・ファイル書き込みはそのロックの外で行います。開始・終了保存・破棄・キャッシュのプレビューと削除も同じディスク用ロックを使い、古い保存が破棄後にファイルを作り直すことを防ぎます。

確定字幕は `SharedTranscriptLine = Arc<LiveTranscriptLine>` で一行ずつ保持します。受け付けた本文と時刻の String は同じ行へ移動し、全文の配列、要約待ちの配列、公開する字幕イベントがその行を共有します。保存・要約・復旧用のスナップショットが配列を保持している間に次の行が追加された場合、配列の copy-on-write は行への参照だけを複製し、過去の全文と時刻を再分配しません。配列の索引のコピーは残るため、その費用は行数に比例します。

完了した要約も `SharedSummaryChunk = Arc<LiveSummaryChunk>` と `LiveSummaryChunks = Arc<Vec<SharedSummaryChunk>>` で保持します。本文・用語・累積白板の所有バッファを一度移動し、保存・モデル入力・複数のスナップショットが同じ段を共有します。新しい要約の追加は `LiveSession::append_summary` に集約し、旧スナップショットが残っていても参照索引だけを複製します。過去の本文・白板の全ノードを LIVE ロック内で深く複製しません。最新要約の通知は一段への参照だけを持ち、通知のシリアライズ待機が全要約の索引や以前の段を保持しません。モデルの直前要約コンテキストも配列の末尾を借用し、使わない白板を複製しません。

要約のキャッシュ JSON・全体応答は従来のオブジェクトと配列の形式で、共有の識別子やラッパーを追加しません。最新要約イベントの沿用白板は、後述する version 付きの通知だけで前段を参照します。以前の JSON を同じ共有要約として復元し、Markdown とモデルの本文・用語・白板は切り詰めません。回帰テストは本文・用語・ノードの所有バッファの再利用、複数の保持済みスナップショットの不変性と各段の共有、旧キャッシュとの JSON 往復、全文の Markdown、最新通知が一段だけを保持して旧履歴を解放することを確認します。

`cargo test --manifest-path src-tauri/Cargo.toml --lib benchmark_append_summary_after_retained_snapshot -- --ignored --nocapture` は 75 段・各白板 100 ノードの旧所有配列と共有配列にスナップショットを保持し、セッション Mutex 内の一段追加を七回、順序を交互に測ります。新しい経路は製品の `append_summary` を使い、通常の最適化なし test profile の中央値は 2026-10-08 のローカル測定で従来 0.954 ms、共有 0.002 ms でした。入力の構築、スナップショットの取得、JSON・Markdown・ディスク・モデル・UI・アプリ全体の CPU/RSS と GPU は計測範囲に含みません。追加後は旧配列の長さ・本文と共有要約の同一性を確認します。

キャッシュの復元と NDJSON の replay も同じ共有行を作り、モデル用の文章、Markdown、時刻の判定は行の文字列を借用します。JSON は従来の `{text, at}` と行配列の形式を保持し、Arc の識別子やラッパーを保存しません。serde の `rc` 機能は依存定義に明示します。回帰テストは入力バッファの再利用、全文・待機行・イベントで同じ行が共有されること、保持した複数のスナップショットの不変性、引用符・改行・絵文字を含む全文の JSON 往復を確認します。既存のキャッシュ、NDJSON、終了保存、要約・Markdown のテストも同じ共有行を使用します。

手動ベンチマークは `cargo test --manifest-path src-tauri/Cargo.toml --lib benchmark_append_after_retained_live_snapshot -- --ignored --nocapture` です。一万行の短い字幕と千行の長い字幕でスナップショットを保持し、製品の録音 ID 照合・セッション Mutex・一行追加の処理を七回測定します。通常の未最適化 test profile を使い、履歴の準備・JSON 変換・ディスク・モデル・UI・アプリ全体の CPU/RSS・GPU を計測範囲に含めません。保持した記録の行数と全文、追加後の全文・待機行の長さは計測後に確認します。

通常の保存間隔は録音ごとの直近の成功から 30 秒です。間隔内の確定行は要求を残し、後続の発話がなくても期限後に保存します。待機には非同期タイマーを使い、強制保存は待機を起こします。失敗時は保存済みの行数・要約数・時刻を進めず、30 秒後または次の要求で再試行します。書き込み中に追加された字幕は、今回保存した範囲とは分けて次回保存します。

講義では初回の確定行から復元可能な JSON の基底を作成し、その後は NDJSON に追加します。要約更新と終了時は全体を基底へまとめ、旧ログを除去します。途中で切れたログの末尾は次回の追加時に修復し、基底に含まれる古い行番号は復元時に無視します。自由ノートは開始時刻を含む Markdown に定期保存します。JSON と Markdown は同じディレクトリの一時ファイルへ書き、同期後に置き換えます。これはプロセス中断や書き込み失敗への対策であり、停電時の完全な永続性を保証するものではありません。

終了・破棄・全体要約の IPC は、操作開始時に表示していた `sessionId` を必須引数として受け取ります。現在の録音を後から選び直しません。終了予約と破棄は、録音 ID の確認と状態変更を同じセッションロック内で行います。破棄は保存用のロックを待った後にも ID を確認し、旧ページから遅れた要求が新しい録音やその保存ファイルを除去しません。全体要約も同じロック内で対象を確認し、必要な配列の Arc 参照だけを取り出してから AI を実行します。

LIVE のマイク開始・再開・停止には `liveSessionId` が必要です。STT の予約もこの ID を保存し、停止要求は caller と録音 ID を一度の STT ロック内で確認します。旧 LIVE の停止は同じ caller の新しい LIVE や、入力を借りている Agent を停止しません。開始は待機前、入場ゲート待機後の借用停止、マイク予約の時点で ID を確認します。短い停止要求・予約中は LIVE のセッションロックを保持し、モデル準備や停止完了の待機では保持しません。通常の Agent は入力 ID、ネイティブショートカットは現在の入力意図と入力 ID で開始を確認します。アプリ終了時は内部の全所有者停止を使います。

主画面の LIVE 終了は `live_finish_session` 一回で行います。以前のフロントエンドの「マイク停止 → 全履歴取得 → 終了 → 再取得」の四回の IPC を除去し、バックエンドが対象を予約してから末尾を確定・保存します。保存済み字幕を含む返却スナップショットを表示に使い、終了直前のフロントエンド情報で AI の必要性や段数を予測しません。通常表示と再読み込み後の表示は、同じ実際のバックエンド段階を示します。これは呼び出し構造の変更であり、アプリ全体の CPU/RSS や GPU 障害の改善率を測定した値ではありません。

デモ録音にも UUID を割り当て、読み込み時に古いアクティブなデモデータへ一度だけ ID を補います。デモの終了・破棄・要約も ID を確認し、非同期のデモ要約処理から戻った後に新しいデモ録音を消しません。保存結果は録音 ID と全文を持つ非アクティブな記録を返します。

録音 ID の回帰テストは置き換え後の終了・破棄・全体要約、保存ロック待機中の置き換え、マイク予約中のロック保持、同じ caller の新しいマイクへの旧停止要求を検証します。これらの新しい Rust テストはマイク・モデル・ユーザー保存先を使いません。`live-session-api.test.mjs` は実際の Tauri invoke wrapper の送信先を代替し、IPC 引数とデモの非同期競合・保存結果・ID 移行を確認します。ネイティブ命令とフロントエンドは同じビルドで更新する必要があります。

終了処理は録音 ID に結びつく予約を持ち、二重終了・保存中の再開や破棄を拒否します。マイク停止中の最後の確定行は引き続き受け付けます。保存失敗時は予約を解放して再試行でき、定時要約も再開します。保存中の定時要約は待機し、要約エラーには録音 ID を付けて別の録音へ表示しません。

終了の段階は Rust の `LiveFinishPhase` に集約し、停止・先行保存・AI 生成・最終保存を表します。段階と `finish_revision` はセッションのスナップショットに含め、更新イベントは録音 ID・段階・版だけを送ります。フロントエンドを再読み込みしても保存中の段階を復元し、開始・再開・二重保存を抑止します。失敗後の予約解除も版を進めるため、同じ録音 ID で再試行した後に届く旧進捗や旧スナップショットが操作状態を戻しません。過去の字幕・要約を受け取る際も、終了状態の版は個別に比較します。

進捗と保存完了のリスナーは時間割や AI 準備の読み込みより先に登録します。再読み込み後は現在の段階を表示し、AI を省略した可能性のある録音に未確認の処理段階を表示しません。保存結果のスナップショットは字幕と要約を保持した非アクティブな記録として返し、使用中のセッションと区別します。

回帰テストは代替ライターによる遅延・失敗・静音状態と、一時ディレクトリへの実際の保存を組み合わせます。セッションロックを保持しない書き込み、静音後の自動保存、強制保存による待機解除、失敗後の再試行、旧録音の拒否、初回の復元と途中で切れたログの修復を確認します。ユーザーの保存先への書き込みや実際の GPU 障害は、このテストには含めません。

要約・ホワイトボードの参照を字幕更新ごとに作り直さず、レイアウトはボードごとに保持します。セッションのスナップショットと周期処理が読む要約間隔は `ai/config/timing.rs` の AtomicI64 に保持します。これらの読み取りはファイル・キーチェーン・設定 IO の Mutex に触れません。録音開始の blocking worker が LIVE と保存のロックを取る前に、`ai_config.json` の非秘密フィールドだけを読み直します。欠落・不正な設定は従来どおり 5 分、短い値は最低 5 分に補正します。

AI 設定ファイルの読み書きは短い IO ゲートで順序を付け、資格情報の操作やモデル処理をこのゲート内で行いません。設定の書き込みが成功した後だけ数値と版を更新し、失敗した保存はキャッシュの値を変更しません。通常の設定読み取りはファイルと版を一緒に取得し、後から新しい保存や録音開始の再読み取りがあれば、古い取得結果をキャッシュへ戻さず、古い設定移行による書き戻しも省きます。アプリ外のファイル編集は次の録音開始または通常の AI 設定読み込みで反映し、ファイル監視は追加しません。この制御は設定ファイルと間隔の順序を対象とし、資格情報を含む全体のトランザクションや設定ファイルの原子的な書き込みを保証しません。

回帰テストは実際の一時 JSON ファイルで、繰り返す数値取得が再読み取りしないこと、遅い再読み取りと保存、保存の IO エラー、古い取得・移行の拒否、途中の書き込みを設定読み取りが見ないことを確認します。製品の設定 IO ゲートを故意に保持した状態でも、実際の LIVE 字幕追加・全体スナップショット・小さい通知が完了することを検証します。ユーザーの設定やキーチェーン、実機のマイク・GPU を使うテストではありません。

分割要約と白板のモデル応答は `live/generation/processing.rs` の blocking worker で解析します。白板の根拠補完と既存の統合・縮退防止も同じ worker で行い、AI の通信待機は async に残します。背景処理への入力は生成開始時の字幕・過去要約の Arc 参照を保持し、worker へ渡すために全文を深く複製しません。生成完了後は要約を書き込む同じ LIVE ロック内で録音 ID を検証し、別録音に結果を追加しません。空の要約は従来どおり pending 行を残す再試行対象です。白板の失敗・不正な応答・極端な縮小では既存の規則で前の白板を保持します。

モデル呼び出し前の AI 設定・資格情報の読み取り、各段階のプロンプト構築、TODO の授業計画・シラバスの SQLite 読み取りは `live/generation/requests.rs` の blocking worker で行います。字幕と要約は取得時の Arc 参照を保持し、遅い準備が後から追加された発話を読み直しません。字幕と要約本文は一つの文字列へ直接書き、行ごとの一時文字列の配列や所有参照の反転用配列を作りません。白板の二段目のプロンプトは最初の要約が成功してから作り、最初のモデル待機中に二段目用の字幕文字列を保持しません。最初の要約は全文、白板の補助字幕は従来の末尾 500 行、全体要約は末尾 24 行、TODO は末尾 80 行を使います。全体要約の短時間・local の省略、資格情報の失敗とフォールバックは既存の規則を保ちます。全体要約は設定・資格情報を一回だけ読み、その同じ所有値を検証してモデルとフォールバックの返答言語に使います。準備中の設定変更によって、一つの要求の provider・生成言語・失敗時の言語が別々の読み取り結果になることを防ぎます。全体要約の本文清掃と TODO の JSON 解析も結果 worker へ移し、TODO は空タイトルを除外する前の六項目という従来の上限を保持します。

生成準備の差分テストは変更前のプロンプト構築をテスト内だけに残し、日・中・英・韓の返答言語、講義と自由ノート、各末尾境界、Unicode・改行・引用符で system/user メッセージ全体の一致を確認します。単一 async スレッドと遅い worker を使い、LIVE と保存のロックが空いていること、別の async task と字幕追加が進むこと、取得済みの字幕の不変性を確認します。実際の一時 SQLite DB から授業計画と評価方法を取得し、モデルやユーザーの資格情報は使いません。結果テストは全体要約の文字列応答、完全な TODO の本文・講義と曜日時限の対応、六項目の上限・既存の失敗処理を検証します。これはスレッドと通信形式の検証であり、実際のキーチェーン待機、モデル、マイク、UI や GPU 障害を再現する検証ではありません。

白板の根拠となる字幕照合は `live/whiteboard/excerpts.rs` で、同じ区間の字幕の空白除去・ASCII 小文字化・原文の文字数を一度だけ計算します。供給済み・前の白板から継承した・用語に付属した引用が使えないノードに初めて遭遇した際に作成し、全文は借用します。処理中だけ必要な正規化文字列を保持するため、その分の一時メモリを使います。録音全体の常駐キャッシュにはしません。用語の得点、原文文字数による同点処理、最後の同点行を採用する順序、80 文字の補完引用と出典優先順位は保持します。

差分テストは従来の照合実装との比較で日本語・ASCII・Unicode・空白・引用の同点・用語数の上限を確認します。背景処理は実際の解析・補完・統合を使い、取得済みの字幕の後の追加、旧白板の不変性、失敗時の旧白板維持を検証します。`cargo test --manifest-path src-tauri/Cargo.toml --lib benchmark_whiteboard_excerpt_matching -- --ignored --nocapture` は 500 行・75 ノードの照合を五回、順序を交互にして比較し、一時索引の作成も計測します。既定の最適化なしテストビルドの CPU 処理だけであり、モデル・マイク・イベント・実機 UI・アプリ全体の RSS や GPU 障害の改善率は計測しません。

2026-10-08 の初回の索引導入時のローカル測定では、照合の中央値は従来 330.615 ms、索引あり 45.814 ms でした。索引の作成を含むこの合成入力の比較であり、モデルの生成時間や実機での改善率には換算しません。その後の借用・分配の変更と直前の索引実装の比較は、以下の独立した測定を使います。

`AppRoot.svelte` は描画エラー時の復旧画面を提供します。macOS の WebKit GPU 停止はこの境界で直接検出できないため、`frontend_health.rs` が表示中の LIVE の DOM 応答を受動的に記録します。診断のために `scripts/diagnose-live-macos.sh` を使えます。プローブは字幕本文を送らず、ウィンドウを自動で再起動しません。

健康プローブの `rootTextLength` は `TreeWalker` で Text と CDATA の既存文字列の UTF-16 長を加算します。以前の `root.textContent.length` と同じ完全長を報告し、非表示の訪問済みページを含む全文を一つの一時文字列へ連結しません。コメントと shadow root は以前と同様に数えず、hidden・script・style と light DOM の文字列は含めます。文字数を打ち切る上限やレイアウト読み取りは導入しません。診断ログの error 数も中間の `filter` 配列を作らずに数えます。30 秒の周期、報告フィールド、欠けた root、復旧画面と本文を送らない規則は保持します。LIVE の活動判定は既存の `try_lock` であり、録音のロックを待ちません。

`tests/frontend-health.test.mjs` の六項目は Rust が WebView へ渡す実際の script と製品 bootstrap を読みます。代替 DOM の全文 getter を禁止し、Unicode・結合文字・不対サロゲート・改行、一万個の既存文字列、空の root、bridge がない場合の無走査と匿名の報告を確認します。凍結した `tests/fixtures/frontend-health-probe-before.rs` を `SELAH_HEALTH_PROBE_BEFORE` に指定すると、全文読み取りを防ぐ三項目が失敗します。これは診断の script の検証で、native WebView の描画や IPC を再現しません。

`node scripts/check-frontend-health-browser.mjs` は実際の DOM と XML の Text/CDATA、非表示要素・script・style・shadow root、空の要素と欠けた root を使う隔離 localhost ページです。native invoke は代替 sink にし、旧 script との完全な報告 JSON の一致も確認します。2026-10-08 の Codex 内ブラウザーで二十六項目が成功しました。一万テキストノード・6,080,000 UTF-16 単位の合成入力で、製品の script は aggregate `textContent` getter を一回も読みませんでした。三回の warmup 後、五回ずつの batch を九回交互に実行した一回当たりの中央値は、旧 script 4.72 ms、現在 0.58 ms でした。小さい混在 DOM は両方とも時計の分解能未満でした。この局所測定は合成 DOM の script と代替 sink だけを含み、入力生成と期待値の読み取りは除外します。実際の LIVE の CPU/RSS/GPU、録音や白画面の解消は測定せず、ブラウザー全体の分配量や heap の減少を示すものではありません。検証後はサーバーを Ctrl-C で停止します。

### 白板の辺ラベルの配置とキャッシュ

LIVE と Markdown の白板表示が共用する `static/whiteboard-layout.js` は、辺ラベルと他の辺の衝突判定に一つの線分一覧を使います。従来はラベルの有無にかかわらず、各辺について他の全線分を新しい配列へコピーしていました。現在は最初のラベルが現れた際だけ一覧を作り、自分の線分を index で除外します。ラベルが最後の辺にしかない場合も、前後の全ての辺を判定対象にします。無ラベルの白板では線分一覧を作りません。

各線分の外接矩形を一回求め、ラベルの候補矩形から完全に離れた線分は精密な交差判定を省略します。境界が接する場合は従来の判定を行います。候補の生成・得点の加算順序・同点時の採用順序・生の辺 index に基づく曲線の向きは保持します。木の配置、ノード・辺・術語の数、根拠と出典、topic の選択、全ての出力座標は変更しません。

型付き wrapper `whiteboardLayout.ts` の WeakMap は不変の白板参照ごとに最大八つの option を保持します。キーはタイトル・外部ラベル・topic ID 配列の JSON tuple とし、文字列内の `|` や ID 内のカンマが別の組み合わせへ誤って一致しないようにします。省略・空の option は同じ既定レイアウトを再利用します。白板が解放された際の回収と失敗結果を再計算しない規則は維持します。

LIVE の選択 topic の配置は白板ページを開いた際だけ計算します。従来は「白板が消えたらページを閉じる」effect が、ページを閉じている間も配置を読み、新しい白板ごとに概覧と選択 topic の両方を計算していました。現在はこの effect と選択 topic の derived の両方が展開状態を先に確認します。概覧の配置は引き続き全 topic を使います。開く操作は展開状態を先に変更してから実際の配置の stage preset を読み、DOM の測定後の自動 fit、再表示時のキャッシュ利用、topic 選択と有効な区間への切替時の展開状態を保持します。展開中に白板がなくなった場合は従来どおり閉じます。この条件はタブや OS の非表示状態を対象とせず、概覧や録音を停止しません。

`tests/whiteboard-geometry.test.mjs` は凍結した旧スクリプトと、backend・legacy・明示階層、topic 絞り込み、術語の折り畳み、循環・欠けた親、無効・自己・逆向き・重複の辺、最初または最後だけのラベルを比較します。全フィールドと JSON 全バイト、topic、入力の不変性を確認します。wrapper のテストは実際のレイアウトで区切り文字を含む option と ID の衝突、同等 option の再利用と八件の上限を検証します。旧スクリプトと合成入力はテスト・独立した比較だけで使い、製品へ含めません。

`node scripts/check-live-whiteboard-browser.mjs` は localhost に隔離した検証ページを起動します。表示された URL を開くと、実際の `Live.svelte` の白板状態・操作・derived・effect・markup、共有レイアウトと `LiveWhiteboardPage` を client として使います。2026-10-08 の 45 検証では、閉じた状態での二十回の白板更新は各回の概覧一回だけ、百回の字幕 snapshot 更新は追加の配置ゼロ、初回の展開は選択 topic 一回でした。測定前の stage preset、実 DOM の測定と fit、topic の個別・全選択、手動 zoom と選択・pan 状態の保持、閉じる・再表示・白板の消去・有効な区間への切替、Escape と展開中の unmount も確認します。pan は合成 fixture から状態を設定して保持を確認し、実際のドラッグ操作は計測しません。概覧は同じ derived を読む検証用の文字列で、録音・モデル・native IPC・アプリ初期化と実機の GPU 障害は使いません。サーバーは Ctrl-C で停止します。

`node scripts/benchmark-whiteboard-layout.mjs` は旧・新の実際の共有 JavaScript を別々の window オブジェクトに読み込み、全出力の一致を確認します。Node の組み込み関数を共用し、warmup 後の九回の交互バッチの中央値を一回の配置あたりで測ります。2026-10-08 の macOS ローカル測定は、18 入力ノード・36 ラベル付き辺が 1.088 → 0.383 ms、75 入力ノード・160 ラベル付き辺が 26.734 → 9.323 ms、同じ入力で 25% の辺にラベルがある場合が 6.636 → 2.015 ms、ラベルなしが 0.124 → 0.069 ms でした。術語ノードは配置の前に chip へ折り畳みます。入力生成、wrapper のキャッシュ、DOM・WebKit、IPC・録音、アプリ全体の RSS と GPU を含む測定ではなく、白画面の解消を証明する結果ではありません。

### 白板の辺ラベル候補の正確な早期終了

`placeEdgeLabel` の候補は従来と同じ七つの法線 offset と五つの接線 offset を同じ順序で調べます。offset の費用、既存ラベルとの重なり、ノードとの重なり、辺との交差の全ペナルティは非負で、最良候補の置換は strict `<` です。そのため初期費用または途中までの合計が既存の最良スコア以上になった候補は、残りを評価しても勝てません。この候補だけの座標・矩形の作成または残りの衝突評価を省きます。同点で後の候補に置き換えず、勝つ可能性のある候補の加算順、辺の原 index と偶奇、bounding box と接触境界・精密判定は保持します。近似の位置・候補の間引き・辺やノードの削除は追加しません。

`tests/whiteboard-label-bound.test.mjs` の四項目は、変更直前の共通 script を独立した `whiteboard-label-bound-before.js` に固定して比較します。通常・legacy・明示階層・backend・循環・欠けた親・重複 ID・prototype 名を含む 185 入力 × 八条件と、遅いラベル・逆向き・重複・不存在の辺を含む別の 32 入力 × 四条件、合計 1608 回の完全 layout の全 JSON byte を比較します。topic・chip・出典、入力の不変性も確認します。2048 組の直接的な衝突条件では選択位置と追加する占用矩形を比較し、正負の法線候補が同点の場合の先勝ちも固定期待値で確認します。

`tests/load-whiteboard-scoring.mjs` はテストと手動測定だけで、実際の private scorer にアクセスし、矩形の作成・重なり判定・精密な線分判定の数を数えます。source の一意な位置にだけ計数を挿入し、製品には計数・追加 export・wrapper を含めません。衝突のない一つのラベルの候補矩形は 35 → 1 個で、選択位置は同じです。密な三条件でも全出力と graph を保って実際の矩形・精密交差の判定が減ることを確認します。旧 scorer ではこの計数の項目だけが失敗し、位置・完全値を確認する項目は同じ結果のまま通ります。

手動比較は `node scripts/benchmark-whiteboard-label-bound.mjs` です。新旧の実際の非計数 script を別の window オブジェクトで動かし、三回の warmup 後の九回の交互 batch の中央値を完全 layout 一回あたりで測ります。正規化・forest・用語の chip 化・全 geometry と label の選択を含みます。入力生成・wrapper の cache・DOM/WebKit・IPC・モデル/録音・結果の比較と破棄・アプリの RSS と GPU は除外します。計数は別の呼び出しで行い、計数の追加費用を時間に含めません。入力と全出力 JSON は両実装で一致します。

2026-10-08 の macOS / Node で、この候補上限変更後の測定時ソースは、18 入力ノード・36 ラベル付き辺が 0.421826 → 0.114565 ms、75 ノード・160 辺が 8.916198 → 2.526078 ms、96 ノード・192 辺が 13.092854 → 3.669472 ms でした。96 ノードの場合の矩形判定は 1199520 → 951517 回、精密交差は 315953 → 60833 回、候補矩形は 6720 → 6666 個です。75 ノードでは矩形判定が 809200 → 661543 回、精密交差が 222645 → 41715 回、候補矩形が 5600 → 5525 個でした。候補矩形の数の減少だけを計算量全体の改善率に扱いません。

75 ノード・160 辺で label が 25% の場合は 1.912125 → 0.465958 ms、精密交差が 50244 → 8381 回でした。label なしでは scorer を呼ばず、すべての判定は従来も現在もゼロ、完全 layout の時間は 0.096824 → 0.090843 ms とほぼ同じです。用語を chip に折り畳む既存の処理により、75 / 96 入力ノードの表示はそれぞれ 65 / 83 structure ノードと全 chip、160 / 192 辺で、両実装に差はありません。全入力の高速化やアプリ全体の速度改善率は主張しません。

ブラウザー検証の追加四項目は、同じ変更直前の script を隔離して読み、96 入力ノードの全 topic を実際の LIVE と Markdown renderer で表示します。概覧と選択 layout の全 JSON byte、すべての表示ノード、すべての辺ラベルの text と百分率の座標、両 SVG の全 path、入力の不変性と実測 viewport の fit を確認します。通知・復旧・ID・選択・字幕・解除を含む 160 項目を確認します。インストール済みアプリやマイクは起動せず、GPU の白画面の原因・解消を証明する検証ではありません。

### 線分交差の方向比較

精密な交差判定の `segmentsCross` は二つの最初の向きが同じ符号なら残りの向きを計算しません。方向値を小さな関数で -1 / 0 / 1 に変換する代わりに、正と負の二つの比較で同じ三分類を直接判別します。zero・負の zero・NaN は正負のどちらにも入らず、元の分類と同じです。候補の順序、矩形との接触判定、スコアとその加算順は変更しません。

`tests/whiteboard-intersection.test.mjs` の四項目は変更直前の共通 script を `whiteboard-intersection-before.js` に固定します。32768 組の生成した方向条件と 72 組の zero・非有限・極小・極大値、4096 組の矩形と線分、通常・legacy・重複 ID・prototype 名と密な board を含む 217 入力 × 八条件の完全 layout を比較します。全 JSON byte、topic・chip・label 座標と入力の不変性を保持します。計数は実際の private 関数にテスト時だけ挿入し、候補矩形・重なり・精密交差の呼出し数は同じで、方向計算だけが減ることを確認します。

`node scripts/benchmark-whiteboard-intersection.mjs` は非計数の完全 layout を三回 warmup 後に九回の交互 batch の中央値で測ります。2026-10-08 の macOS / Node では、75 入力ノード・160 ラベル付き辺が 2.354049 → 1.936278 ms、96 ノード・192 辺が 3.542182 → 2.873401 ms でした。96 ノードで方向計算は 670168 → 379046 回ですが、精密交差 60833 回、候補矩形 6666 個、全表示ノード・chip・辺は同じです。75 ノードで label 25% は 0.423867 → 0.367401 ms、18 ノード・36 辺は 0.087822 → 0.085591 ms、label なしは 0.082877 → 0.082074 ms で、小さい入力や label なしの利得は小さい値です。全入力の高速化率やアプリ全体の改善率には扱いません。

入力生成・wrapper cache・DOM/WebKit・IPC・モデル・録音・アプリ RSS/GPU と結果比較を時間に含めません。計数は別の呼出しです。最終ソースを使う隔離 browser でも実際の LIVE と Markdown component の 160 項目を確認しました。実機の GPU 障害や白画面の解消を証明する検証ではありません。

### 保存済み白板の表示 ID と索引

`static/whiteboard-layout.js` の `topics` と `compute` は、全ての有効な入力ノードに同じ表示 ID の処理を適用してから主題の選別・術語の折り畳みを行います。旧キャッシュや Markdown は、backend 正規化済みでも重複 ID を含む場合があります。元の ID の最初のノードはその ID を保持し、後の重複・欠けた ID にだけ一意な表示 ID を付けます。後から現れる元の ID を最初に全て予約し、生成した接尾辞がそのノードの ID を奪わないようにします。元の parent・辺の参照は最初の元 ID のノードを指します。重複した元 ID の参照先の意図は復元できません。保存値を書き換えたり、重複したノード・全文を削除したりはしません。

ノード・子・座標・主題選択・術語の親・辺の隣接関係の ID 索引は、prototype のない辞書を使います。`constructor`、`__proto__`、`toString` なども通常の ID として扱い、存在しない親が内蔵プロパティを指すことはありません。従来の普通のオブジェクトでは、存在しない `constructor` 親の子リストに `push` できず例外が出たり、孤立した術語が `Object.prototype` に付加されたりしました。この修正は LIVE と Markdown の共通層に置き、表示側に別々の修復ルールを追加しません。

`tests/whiteboard-identity.test.mjs` の六検証は、変更前には全て失敗しました。旧 ID の接尾辞衝突、予約済み接尾辞、欠けた ID・数値 0、特殊な名前、存在しない特殊な親、legacy・明示階層・backend 形式を確認します。入力の不変性、主題と配置の ID の一致、全ノード・術語・参照の保持と有限な座標を検証します。既存の 181 入力・八条件では、修正前と普通の白板の全出力 JSON が一致します。`node scripts/check-live-whiteboard-browser.mjs` は実際の LIVE の状態・page と Markdown component を分離したブラウザーで実行し、両方の主題切替・各ノードの独立した選択・本文と術語・辺・字幕更新・unmount を確認します。

`node scripts/benchmark-whiteboard-layout.mjs --identity` は、この修正の直前の共有実装だけと比較します。2026-10-08 の macOS/Node 測定では、18 入力ノード・36 ラベル付き辺が 1.060 → 1.193 ms、75 入力ノード・160 ラベル付き辺が 24.040 → 24.036 ms、同じ入力でラベルが 25% の場合が 5.293 → 5.273 ms、ラベルなしが 0.163 → 0.220 ms でした。小さい配置では ID の予約と安全な索引に追加費用があります。共有 wrapper のキャッシュにより、同じ白板を使う字幕更新では再配置しません。九回の交互バッチの中央値であり、入力生成・キャッシュ・DOM/WebKit・IPC・録音・アプリ RSS・GPU を含みません。これは保存済みデータによる配置例外の修正であり、実機の GPU 障害と白画面の解消を証明する検証ではありません。

### 白板の共有スクリプトの読み込み

`whiteboardLayout.ts` は import だけではスクリプトを挿入しません。LIVE の有効な区間に白板が到着した時と Markdown の白板 component の effect が `prepareWhiteboardLayout` を呼びます。空の LIVE や白板のない Markdown の import では共有配置の JavaScript を要求しません。derived が読む白板参照は通常の字幕更新で同じなので、その更新ごとに読み込みを再要求しません。

`ensureWhiteboardLayout` は window 内の一つの未完了 promise を保持し、並行する表示からの要求を共有します。DOM に触れる前に promise を登録し、挿入などの同期失敗も reject として返します。load / error のどちらで終了しても両方の listener と未完了 promise を解除します。失敗した script は取り除き、再試行は新しい script を使います。未完了の所有要求がない古いタグも置き換え、既に終了したイベントを待ち続けません。終了した attempt の callback が後から実行されても、新しい promise と準備状態を変更しません。正常終了には compute / topics の両方の関数が必要です。

`prepareWhiteboardLayout` は失敗後に一回だけ再試行します。複数の表示は再試行の実際の要求も共有し、二回とも失敗した場合は表示側へ error を返して停止します。無限の polling や retry timer は導入しません。後の新しい白板または component の再表示は改めて要求でき、既に正常な engine があれば network 要求なしで再利用します。共有スクリプトの寿命は window 単位であり、最後の白板を閉じた際には engine を unload しません。準備状態の store は遅い読み込みの完了後に実際の derived を再評価します。

`tests/whiteboard-loader.test.mjs` の九検証は失敗後の復旧、百要求の合流、欠けた export、旧 callback、DOM の同期失敗、古いタグ、非ブラウザー・既存 engine の省略、二回までの再試行と後の新しい要求を確認します。変更前の失敗→再要求の検証は、旧タグの終了済み error イベントを待って新しい script を作らないため失敗しました。

`node scripts/check-whiteboard-loader-browser.mjs` は二回だけ asset を 404 にする localhost 検証を起動します。実際の Markdown 白板二個、共有 wrapper と配置スクリプトで、import 時の要求ゼロ、二個の表示の要求合流、二回での停止と失敗タグの除去、次の白板で一回の要求による復旧、準備完了後の両方のノードと完全なタイトル、百回の無関係な更新、閉じる・再表示・unmount の十六検証を行います。LIVE の隔離検証も読み込み入口を含む実際の状態を使い、四十五検証が成功しました。いずれも録音・native IPC・モデル・アプリ初期化や GPU 障害は使用しません。検証サーバーは Ctrl-C で停止します。

### LIVE の要約・用語カードの表示範囲

右側の摘要と用語のデッキは `views/live/liveDeck.ts` で循環する表示範囲だけを選びます。以前は全項目を DOM に生成し、先頭と背後の二枚以外を `visibility: hidden` にしていました。現在はこの三枚だけを mount し、摘要の Markdown も表示範囲に入った時点で既存のサニタイズ済み renderer に渡します。元の全文・要点・用語配列と詳細ページは保持し、表示枚数で内容を切り捨てません。

共通関数は最大三項目だけを参照し、以前と同じ元の index・循環 offset・DOM 順序を返します。既存のキー、前面と背後の位置、用語の前後・背後カードの選択、全項目を巡回する摘要の 4.5 秒タイマーを保持します。選択する index は表示範囲内の位置ではなく元の配列の index です。通常の字幕差分は摘要・白板の参照を保持し、同じデッキの Markdown 再描画やタイマー再登録を引き起こしません。新しい項目が背後へ入る際はそのカードを生成し、表示範囲を離れたカードを破棄します。

`tests/live-deck.test.mjs` は凍結した旧表示式と空・一枚・少数・多数・範囲外の一時 index を比較し、一万項目の全てが前面へ巡回でき、元のオブジェクトを借用することを確認します。実際の Svelte を server 向けにコンパイルした差分テストは、用語の全循環位置で可視本文・出典・inline の位置と opacity・ARIA・tabindex・ナビゲーションを比較します。128 要点の初期描画は 128 → 3 枚・Markdown renderer 呼び出しになり、本文全体と詳細入口は保持します。これは DOM を作る前のテンプレート出力の検証であり、実機の GPU/RSS の測定ではありません。

`node scripts/check-live-decks-browser.mjs` は localhost の隔離ページを用意し、実際の client 向けコンポーネントと DOMPurify、CSS、Svelte の反応性を使います。表示された URL をブラウザーで開くと、107 個の確認結果をページと端末に出します。12 要点を二周、100 回の親更新と字幕 snapshot の置換、同じ内容での参照置換、新しい摘要、用語の前後と一周の境界での背後カードのクリック、キーボードの詳細入口、消去・再 mount・unmount を確認します。タイマー・時計・描画 frame はページ内だけで手動で進めます。検証する表示時計と字幕追従の binding は製品の `Live.svelte` の `bindLiveDisplayClock` と `bindLiveTranscriptFollow` をそのまま読み、状態入力だけを隔離ページから渡します。非表示・表示復帰・詳細ページによる被覆、非表示前に予約された旧 callback の拒否、時計の即時更新と実際の DOM のスクロールも確認します。実際の待ち時間・OS ウィンドウの最小化・LIVE IPC・マイク・AI・画面全体の描画負荷や白画面は再現しません。2026-10-08 の Codex 内ブラウザーで全確認が成功しました。終了後は端末でこの検証サーバーを停止します。

### LIVE 起動操作の所有

`startSession` は readiness と native の会話作成を待った後にも、ページの scope、録音 ID、活動状態と後端の finish reservation を確認します。終了したページから遅い作成応答を表示したり、新たに `stt_start_stream` を発行しません。既に発行した native の作成や音声開始は取り消さず、後端の recording と記録データは次のページの復旧へ残します。ページ移動で進行中の録音を停止・破棄する処理ではありません。

初期音声の失敗清理は `pendingStartSessionId` で具体的な recording に結び付けます。別 recording の STT エラー、旧コマンドの遅い失敗、後端が既に listening を通知した要求から記録を破棄しません。現在の初期化エラーは従来の表示を保持して自身の ID だけを cancel します。STT エラー通知が先に取消を所有した場合、コマンドの後の拒否では同じ取消を重ねません。取消を待つ間に別録音が始まれば、旧 cleanup から復旧読み取りを発行せず、新しい静音 deadline や字幕を消しません。

`tests/live-start-owner.test.mjs` の十四項目は実際の開始・readiness loop・snapshot merge・STT phase と購読関数、製品の `ResourceScope` / `ResourceSlot` を使います。reactive busy は getter、readiness・IPC・UI 通知は代替境界です。百開始の既存 busy gate、readiness 中と作成中の終了、別 recording と finish reservation、発行済み音声の遅い応答と失敗、現在のエラー、listening 通知、他録音の STT エラー、重複取消、解除後の通知、demo と disabled readiness、二種類の取消待機中の別録音を確認します。これは async 操作の所有と要求数の検証で、Svelte の実 DOM、native 録音・マイク・データ削除、CPU/RSS/GPU や白画面の解消を測定するものではありません。

### LIVE の講義キャッシュ清理とプレビューの所有

`executeClearCourseData` は発行時の講義・プレビュー identity・表示 snapshot・保存済み preview・session event の版を保持します。native の削除完了や失敗の後は、ページが生存し、これらの所有が同じで録音中でない場合だけ、表示を空にするか通知を出します。別講義や自由ノートへの変更、録音の開始・終了、新しい復旧 snapshot や保存結果を旧清理の応答で上書きしません。既に表示されている保存 badge は清理を妨げません。native に発行した削除は取り消さず、元の講義を指定したまま完了を待ちます。

清理の開始時に `LatestViewRead` の古いプレビューを無効にし、削除前に読んだ内容を削除後へ戻しません。`bindLiveCoursePreview` は講義と日付の primitive identity と、録音中・保存 badge・UI 操作中でないという primitive gate を監視します。操作中に選択が変われば読み取りを延期し、空閑へ戻った時に現在の選択を一回読みます。自由ノートへの変更や gate の停止、ページ破棄で旧読み取りを無効にします。同じ identity の講義オブジェクトの再生成、同日内の時計と非活動中 snapshot の置換では再読しません。清理後の一回の読み取りは削除後のキャッシュの確認も兼ねます。

`tests/live-clear-owner.test.mjs` の十一項目は実際の清理関数、講義 identity と DTO、プレビュー適用処理と `LatestViewRead` / `ResourceScope` を使います。native IPC と reactive 入力は代替境界です。応答前の内容保持、削除前の読み取り、講義・自由ノート・録音・復旧・保存結果・日付の変更、破棄、百回の重複操作、現在の失敗と再試行、既存の保存 badge を確認します。所有の変更直前のソースでは最初の十項目中六項目が失敗し、修正後は全て成功しました。追加項目は後述の native IPC fixture の文字列エラーと null 応答を再生します。ユーザーの実データ削除は行いません。

`node scripts/check-live-clear-owner-browser.mjs` は実際の client 向け Svelte の講義選択・identity・preview binding・清理関数を使う隔離 localhost ページを起動します。2026-10-08 の Codex 内ブラウザーで二十七項目が成功しました。成功と失敗の後の選択補読、同一講義への復帰、旧 preview の拒否、新録音・保存 badge と新保存結果、百回の同一 identity の更新、翌日と unmount の DOM を確認します。状態と遅延 IPC は代替境界で、native の削除・録音・マイクや実機の GPU/RSS・白画面は検証しません。検証サーバーは Ctrl-C で停止します。

### LIVE キャッシュ削除の失敗通知

明示的な `live_clear_day_cache` は `live/commands/clear_cache.rs` の blocking worker で保存 gate を取得した後、その時点の録音を再確認します。録音中または保存中の同じ講義は削除しません。別講義の削除 IO は LIVE の session lock を保持せず、保存 gate は保持します。自由ノートの no-op、空講義名のエラーと成功時の null 応答を保ちます。

`live/cache/removal.rs` は主 JSON と字幕ログを順に削除し、`NotFound` だけを成功として扱います。主 JSON の削除が失敗すればそこで停止し、まだ主 JSON に含まれていない発話のログを追加で消しません。字幕ログの削除失敗も文字列エラーとして IPC に返し、前端が成功通知を出すことを防ぎます。二ファイルの削除は原子的ではなく、後のログ削除が失敗した場合に既に削除した主 JSON を復元する保証はありません。取消と空録音の終了では従来の制御フローを保持し、残ったキャッシュの清掃失敗を native log に記録します。

削除の四テストは一時ファイルで全ての有無の組み合わせ、繰り返し、正式 Markdown の保持、実際の directory に対する削除失敗と追加ログの保持・再試行、二番目の失敗、注入した権限・IO エラーを確認します。凍結した旧二行の削除では、主 JSON と同名の directory に対して失敗してもログを消すことを同じ一時入力で再現します。worker の五テストは validation、現在の録音と finish の保護、別講義 IO の lock 範囲、単一 async スレッド上の保存 gate 待機と待機中の新録音、IO・panic・poison と再試行、MockRuntime の IPC 応答を確認します。

`tests/fixtures/live-clear-cache-wire.json` は同じ domain worker と製品削除関数を使う test command の実際の IPC 結果です。test command ではパスを一時 directory に置き換え、最初だけ filesystem の権限エラーを代替境界から返します。成功の再試行は実際の一時ファイルを削除します。fixture はプロジェクトルートで `SELAH_CLEAR_CACHE_WIRE="$PWD/tests/fixtures/live-clear-cache-wire.json" cargo test --manifest-path src-tauri/Cargo.toml --lib live::commands::clear_cache::tests::native_ipc_preserves_error_strings_and_success_null_and_retries_actual_temporary_files` を実行して再生成できます。これは失敗の通知と worker/保存の順序の検証で、ユーザーのダウンロード先・マイク・native WebView や GPU 障害は使いません。

### LIVE の表示タイマーの寿命

Dashboard は訪問したページを保持し、選択していないタブを CSS で非表示にします。LIVE は既存の `activeTab` と `document.hidden` の監視から表示状態を導出し、摘要の 4.5 秒の輪番と表示用の 30 秒の時計だけをその状態で制御します。非表示では両方を解除し、白板・摘要の詳細ページが右側カードを覆う場合は摘要の輪番だけを解除します。表示に戻ると現在の時計を直ちに更新し、摘要の現在位置を保って各タイマーを一つだけ作成します。録音・保存と 60 秒の無発話/休止の自動終了チェックはこの表示条件に含めず、従来どおり背景で続きます。

時計の稼働状態と摘要の項目数・内容 fingerprint は primitive の derived を使います。字幕による snapshot オブジェクトの更新や同じ内容の摘要参照の置換で、タイマーを解除・再登録しません。`ResourceScope.interval` は repeating timer を所有し、解除時は `clearInterval` より先に callback を無効にします。既にキューへ入った旧 tick が表示復帰後の新しいタイマーに混ざらず、解除・scope の破棄は重複しても一回だけ処理します。scope の破棄後は新しい interval を作りません。リソースのテストは解除後・破棄後・新しいタイマーの開始後の古い callback を明示的に実行して無効であることを確認します。この変更は全ての非表示 DOM 更新を停止する機構ではありません。

保存済みの表示（6 秒）と成功通知（4 秒、個別通知は指定時間）は `ResourceScope.schedule`、講義の時刻更新と録音の自動チェック（各 60 秒）は `ResourceScope.interval` が所有します。置換・非表示・録音終了・ページ破棄では該当する cleanup を呼び、予約済みの旧 callback も無効にします。同じ文面の通知へ置き換えた場合にも、古い通知の callback が新しい通知や cleanup の所有を消しません。講義の時刻更新は表示に戻った時に即時実行します。録音チェックは primitive の `snapshot.active` の derived で起動し、同じ活動状態の字幕更新では effect 本体を再実行しません。背景の録音チェック、10 分の無発話による休止、20 分の休止による終了と処理中の重複防止は保持します。

`tests/live-timers.test.mjs` の九項目は製品の関数・タイマー宣言・破棄処理と `ResourceScope` を読み、百回の表示置換、同文通知、永続エラー・readiness 通知、表示復帰、別録音、破棄後の予約 callback、処理中の重複と既存の時間条件を確認します。時計・scheduler・IPC/音声操作・reactive 入力は代替境界です。変更直前のソースでは九項目中五項目が失敗し、修正後は全て成功しました。

`node scripts/check-live-timers-browser.mjs` は実際の client 向け Svelte、製品の timer binding と破棄処理を使う隔離 localhost ページを起動します。表示される URL を開くと、同文通知と保存表示の DOM、非表示と復帰、百回の活動中 snapshot 更新での同一 timer と effect 本体の省略、背景の休止チェック、旧録音 callback、四種類の timer を残しての unmount を確認します。2026-10-08 の Codex 内ブラウザーで二十項目が成功しました。時計と scheduler、状態データ、音声操作はページ内の代替境界で、native IPC・マイク・録音や OS の描画は使いません。CPU/RSS/GPU の改善や白画面の解消を示す検証ではありません。検証サーバーは Ctrl-C で停止します。

### LIVE の字幕スクロールの所有権

字幕の自動追従は `views/live/liveTranscriptFollow.ts` に分離し、録音 ID・DOM 対象・行数と、表示中・聴取中・自動追従の三条件を受け取ります。`Live.svelte` の対象要素の binding も `$state` にし、要素の到着・入れ替えを追従制御へ通知します。最大一つの `requestAnimationFrame` を予約し、待機中の字幕は最新の行数へ更新するだけで、転写本文や過去の行を保持しません。frame が実行された時に現在の DOM の `scrollHeight` を一回読み、末尾へ追従します。既存の同一 frame 内の合流と、行数が同じ仮字幕の更新で新しい frame を作らない規則は維持します。

非表示・詳細ページによる被覆・聴取の停止・手動スクロール・録音または要素の変更では予約を解除します。解除は scheduler の取消より先に現在の frame を無効にし、遅れて実行された旧 callback が新しい frame を消費したり、新しい録音の DOM に触れたりしません。追従済みの行数は予約時ではなく、実際のスクロールの成功後に記録します。追従を再開した際は行数が同じでも一回だけ現在の末尾へ追いつき、非アクティブな保存済み記録では自動追従しません。ページの `ResourceScope` が終了時に予約と DOM 参照を解放します。手動の「最新へ」ボタン、全文保存と 120 行の表示範囲は変更しません。

`tests/live-transcript-follow.test.mjs` は一万回の字幕更新が一つの予約・一回の高さ読み取りになること、同じ行数の更新で追加予約がないことを確認します。非表示・休止・手動スクロール後、同じ行数での再開、録音・DOM 対象の入れ替え、破棄後の旧 callback、scheduler と高さ読み取りの失敗後の再試行も確認します。これは局所的な予約とレイアウト読み取り回数の検証であり、以前も表示中の同一 frame 内の更新は合流していました。アプリ全体の速度改善率へ換算しません。

隔離ブラウザーでは実際の Svelte binding と対象 DOM を使い、120 回の字幕更新の一つの予約、最新の高さへの追従、手動位置の保持、非表示と被覆・休止からの再開、旧録音の予約が新しい予約に混ざらないこと、未実行 frame を残しての unmount を検証します。fixture の転写は製品と同じ 120 行まで表示しますが、native の保存・録音・IPC は使いません。fixture の frame は手動で実行し、スクロール位置を即時確認するため smooth scrolling は使いません。実際の smooth scrolling の途中の動きや OS の描画負荷はこの検証に含みません。

### LIVE キャッシュのストリーム書き込み

`live/cache/encoding.rs` は完全なキャッシュ JSON と追加する NDJSON の各行を 64 KiB の BufWriter でファイルへ直接変換します。保存する全文や追加バッチ全体の Vec を先に作りません。保存 DTO は `live/cache/types.rs` に集約し、フィールドの順序と形式、全文・用語・白板、行番号と時刻を保持します。保存 worker とディスク用ロック、30 秒の周期、成功後だけ進める保存進捗は共用します。

完全なキャッシュは同じディレクトリの新規一時ファイルへ変換し、flush と sync_all、ファイルを閉じる処理の後に置換します。書き込み・同期・置換の失敗と変換中の panic では一時ファイルの所有ガードがファイルを閉じてから除去し、以前の完全なキャッシュを保持します。古い NDJSON の除去は置換の成功後だけ行います。変換エラーとファイル IO エラーは区別して元の詳細を返します。

NDJSON の追加は従来と同じ末尾修復の後の長さを記録し、全行を flush・sync_data した場合だけ確定します。失敗・panic 時は writer のバッファを先に破棄し、その後で新しいバッチを記録した長さへ戻して同期を試みます。ロールバック自体の IO 失敗や停電に対する完全な保証はありません。追加する行がない場合はログを開きません。基底がない場合は空の追加範囲でも全体の JSON を保存します。末尾修復は欠けた最後の一行を保持して解析し、復元は JSON と完全な所有データを保持するため、保存・復元全体のメモリが 64 KiB に制限されるわけではありません。

一時ディレクトリで製品の保存経路を使う差分テストは、空・境界・一万行のキャッシュと追加ログの旧エンコーダーとの全バイト一致、要約・用語・白板と全文の復元、非常に長い Unicode 行を確認します。部分書き込みと flush の失敗、途中まで変換した後のエラーと panic、置換失敗、末尾修復後の失敗を注入し、旧ファイルの保持、一時ファイルの清掃、追加バッチの巻き戻しと重複しない再試行を検証します。ユーザーの保存先とマイクは使いません。

`cargo run --manifest-path src-tauri/Cargo.toml --example benchmark-live-cache` は製品 DTO とエンコーダーを使い、以前の全体 Vec と固定バッファの出力サイズ・SHA-256 を比較します。通常の最適化なし dev profile で入力準備を除外し、ウォームアップ後に九回の交互実行の中央値を測り、分配量は別の呼び出しで計測します。`examples/support/allocation.rs` の allocator は独立した比較プログラムだけが使い、製品には導入しません。

2026-10-08 の macOS ローカル測定では、一万行の JSON は 1,231,211 bytes、境界での生存要求量のピークは 2,097,152 → 65,536 bytes、分配要求は 15 → 1 回でした。同じ字幕の追加ログは 1,227,780 bytes、2,064,384 → 65,536 bytes、18 → 1 回でした。五万行では JSON が 8,388,608 → 65,536 bytes、ログが 8,257,536 → 65,536 bytes でした。小さい出力には固定バッファが以前より大きくなる場合があります。これは変換と SHA-256 の受け手だけの合成比較で、入力・allocator 管理領域・realloc 内部の重複、ディスク・同期・置換・末尾修復、IPC と実機 UI、アプリ全体の RSS・GPU は計測しません。

### LIVE キャッシュの復元メモリ

開始と日別プレビューが共用する `live/cache/recovery.rs` は、基底 JSON の解析後にその原文 String を解放してからログを開きます。JSON は従来の Serde の文字列パーサーを使い、本文・用語・白板と完全な所有データを復元します。NDJSON は 8 KiB の BufReader と再利用する一行の String で読み、ログ全体の原文を保持しません。字幕の上限や文字数による切り詰めを導入せず、非常に長い一行ではその長さに応じた一時バッファを使います。

NDJSON の `LiveLineDeltaBorrowed` は本文と時刻を `#[serde(borrow)]` 付きの Cow として解析し、エスケープのないフィールドを現在の入力行から借用します。行番号を確認してから、復元する新しい行だけを所有文字列へ変換します。既に基底に含まれる行と重複行は、その本文・時刻を新しく分配せずに破棄できます。改行・引用符・Unicode escape などは従来どおり完全に復号し、Serde が作った所有文字列を新しい行へ移動します。末尾の修復前の完全性検証にも同じ借用 DTO を使います。入力行への参照は解析中だけに存在し、保存した字幕が再利用する読み取りバッファを参照しません。

復元する行は基底の配列へ追加し、読み取りの完了時だけ回放を確定します。IO・UTF-8 の失敗と panic は追加前の長さまで戻し、拡張した参照索引の容量も元の容量を下限として縮小します。基底の行と要約を保持し、別の段階用に完全な行の配列を複製しません。空白・不正 JSON・古い行番号を無視し、最初の行番号の欠落から先は復元しない規則を保ちます。欠落後もファイルの終端まで読み取りと UTF-8 検証を続けるため、後半の不正 UTF-8 や IO 失敗も以前と同様にログ全体を無視します。完全な末尾行は改行がなくても復元し、壊れた JSON の末尾は無視します。日付や講義名が違う基底の清掃、欠けた・不正な基底を使わない挙動、読み取れないログでは基底を返す挙動も保持します。保存ゲートと背景 worker、ファイル形式は変更しません。

凍結した従来処理 `live/cache/recovery/before.rs` はテストと独立した比較プログラムだけに含めます。差分テストは空・境界行数、1〜8192 bytes の読み取りバッファ、CRLF・改行なしの末尾、空白・不正な型・未知フィールド・重複行と行番号の欠落、非常に長い Unicode 字幕を比較します。途中まで読んだ後と欠落後の IO 失敗・不正 UTF-8・panic では基底へ戻すことを検証します。一万行のログの最後で UTF-8 が壊れた場合も本文と拡張した参照索引を保持しないことを確認し、実際の一時ファイルで復元の全フィールド、元ファイルの保持と古い記録の清掃を比較します。

借用 DTO の差分テストは以前の所有 DTO と、通常の Unicode・全種の JSON escape・フィールド順序・未知フィールド、欠落・不正な型・重複フィールド・範囲外の行番号・不正 UTF-8 の受理と復号結果を比較します。以前の Serde が受け付ける三要素の配列も同じ結果になります。通常フィールドが入力内を参照し、エスケープの所有フォールバックが複製なしで移動することも確認します。所有 DTO は製品ビルドには残さず、従来処理の比較には固定した型を使います。

`cargo run --manifest-path src-tauri/Cargo.toml --example benchmark-live-recovery` は実際の製品ローダーと従来処理で一時ファイルを読み、復元した全 JSON のバイト一致と同じ所有データ量を確認します。通常の最適化なし dev profile、ウォームアップ後に交互実行した九回の中央値で、ファイルの読み取り・UTF-8 検証・JSON 解析・回放を含めます。fixture とファイル作成、結果の破棄・比較は時間計測の外で、分配量は別の呼び出しで測ります。ファイルシステムのキャッシュが温まった局所測定で、コールドストレージの待機やアプリ・マイク・モデル・IPC・実機 UI・RSS・GPU は計測しません。

2026-10-08 の macOS ローカル測定で、エスケープ付きの一万行のうち 3333 行を基底 JSON、残りをログに置いた記録は、境界での生存要求量のピークが 3,000,282 → 1,744,043 bytes、中央値が 21.014 → 21.490 ms でした。五万行・基底 16666 行は 14,686,405 → 8,394,549 bytes、106.265 → 110.748 ms でした。この入力では読み取り時間が約 2〜4% 増え、復元中の一時メモリを減らします。エスケープのない新しい字幕の一万行は 2,390,238 → 1,493,971 bytes、14.564 → 14.952 ms でした。新しい字幕には所有文字列が必要で、分配回数は以前とほぼ同じです。

全行を既に基底へまとめ、ログだけが除去されずに残ったケースでは、エスケープのない一万行の復元全体の分配要求が 61,769 → 41,771 回、中央値が 26.773 → 26.183 ms でした。五万行では 304,773 → 204,775 回、130.528 → 128.700 ms でした。入力原文を全て保持した従来処理との比較で、現在の結果には先行するバッファ読み取りの変更も含みます。借用は破棄する各行の本文・時刻の二つの分配を除去し、読み取りバッファ用の固定分配が残ります。ログが空で全行を基底に置いた場合は JSON 解析時のピークが支配し、ピークの改善はありません。要求量には完全な復元結果も含み、入力の常駐メモリ・allocator 管理領域・realloc 内部の重複は含みません。

### LIVE の Markdown 構築

定期保存と終了保存が共用する `live/markdown.rs` は、メタデータ・全体要約・最新の累積白板・区間要約・用語・転写を同じ出力 String に追加します。行ごとの `format!`、完全な転写の中間 Vec と join、各用語・ノード・辺・区間ごとの文字列と合成文字列を作りません。転写は本文と時刻の UTF-8 バイト長と既存の区切りから必要容量を計算し、追加前に一度だけ予約します。全文転写と他の節を別の大きなバッファに保持した後で結合する処理を除き、行数に比例した文字列の分配を避けます。

戻り値は引き続き全文の Markdown String で、ファイル名、原子的な置換、保存順序、保存周期とロックの範囲は変更しません。用語の原文と外部出典、白板の構造化 JSON fence とテキストによる補足、節と行の区切りと末尾改行を保持します。白板の JSON は既存の Serde で一度変換し、出力へ追加した後に一時バッファを破棄します。辺の参照は従来と同じ最初の一致ノードを使い、存在しない端点を除きます。最後の白板が空の場合に古い白板へ戻る変更は加えません。

差分テストは変更前の実装を `live/markdown/before.rs` に固定し、252 組の講義と自由ノート、空・空白・全文のメタデータ、空から 5000 行の記録、複数の要約と空の概要で出力全体のバイト一致と入力の不変性を比較します。白板の空・一個・多数のノード、空タイトル、重複 ID、存在しない端点、外部と録音内の根拠、空白の判定、最新の空白板と JSON の復号も確認します。長い一行、改行・引用符・絵文字・NUL と空の要約項目も全文を保ちます。凍結した実装は製品ビルドには含めません。

`cargo run --manifest-path src-tauri/Cargo.toml --example benchmark-live-markdown` は製品の型と Markdown ソースを読み込む独立した比較プログラムです。アプリ、マイク、モデル、ユーザーの保存先と IPC を起動しません。タイミングは通常の最適化なし dev profile で、fixture の準備と出力の破棄・比較を除き、ウォームアップ後に新旧を交互にした九回の中央値を表示します。別の呼び出しで Rust の System allocator への分配・再分配要求数、総要求バイト数、呼び出し境界で同時に生存する要求バイト数を計測します。この allocator は比較プログラムだけにあり、製品とライブラリのテストには導入しません。計測終了時は戻り値の String の容量だけが生存することと、新旧の出力バイト一致を確認します。

2026-10-08 の macOS ローカル測定で、一万行・50 段・987,169 bytes の全文は 2.926 → 1.893 ms、分配要求は 32,487 → 28 回、境界での生存要求量のピークは 2,972,551 → 987,169 bytes でした。五万行・250 段・4,824,319 bytes は 11.504 → 6.432 ms、158,288 → 29 回、14,484,001 → 4,824,319 bytes でした。これは合成入力の Markdown 構築だけの計測です。入力の常駐メモリ、allocator の管理領域、realloc 内部の一時的な重複、ネイティブな別の分配、保存と IPC/JSON、実機 UI、アプリ全体の RSS と GPU 障害の改善率は含みません。

### LIVE ページの転写表示状態

`liveSessionApi.ts` の `LiveSurfaceSnapshot` は録音の metadata・正確な総行数・直近 120 行・要約を保持します。LIVE ページが従来から表示する 120 行の窓をそのまま使い、表示しない転写行と待機行の配列をページ状態へ残しません。開始・復元・授業のプレビューと終了保存は下記の表示用応答を受け取ります。元の完全な `LiveSessionSnapshot` は API とバックエンドの保存形式として保持し、デモと完全な応答の互換変換には共通の `liveSurfaceSnapshot.ts` を使います。転写全文と待機中の本文、キャッシュ、Markdown、AI の入力は引き続きバックエンドが管理します。

一行の確定イベントは総行数と録音 ID を検証し、新しい窓に直近 119 行と一行を追加します。古い表示状態と元の行オブジェクトを変更しません。追加でコピーする表示行の範囲は録音の長さに依存せず、要約と白板は同じ参照を保持します。metadata と保存段階だけの更新は窓の配列も再利用し、Svelte の表示窓はこの配列を直接参照します。行数、表示を省略した行数、スクロール追従と開始・終了操作の判定は総行数を使います。待機本文は UI が使用しないため複製せず、要約が消費した先頭位置だけを保持します。保存の表示プレビューは全体要約の Markdown だけを持ち、保存結果の完全な snapshot と保存ファイルの全文を別途保持しません。

更新 revision と保存 revision、録音 ID、連続した行数で旧イベントを拒否し、欠落したイベントと要約は従来どおり全体の復元を要求します。古い復元読み取りでも不足行を含む場合は総行数と窓を回収し、より新しい要約の消費位置と保存段階を維持します。終了通知は録音中の状態を解除し、完了済みの表示・授業プレビューを後続の空読み取りで消しません。新しい録音の状態には旧録音を混ぜません。

回帰テストは変更前の完全配列の処理を `tests/fixtures/live-transcript-before.ts` に残し、2500 行と遅延要約、重複・別録音・欠落イベント、古い復元、保存段階、終了・再開の後の表示を比較します。20000 回の追加で正確な総行数、120 行の上限、保持済みの古い状態の不変性と要約・白板の参照を確認します。一万行の受信 fixture では表示状態のオブジェクトグラフが元の完全配列・待機配列と表示外の行を参照しないこと、入力の JSON と全行が変わらないことも確認します。保存プレビューも同じ方法で完全な snapshot を保持しないことを検証します。

手動測定は `node --expose-gc scripts/benchmark-live-surface.mjs` です。2026-10-08 の macOS・Node、各 run の前に GC、ウォームアップ後に新旧の順序を交互にした九回の中央値で、500 行の追加と窓の読み取りは既存 1000 行で 1.076 → 0.086 ms、10000 行で 6.776 → 0.098 ms、50000 行で 107.772 → 0.086 ms でした。fixture 準備、IPC/JSON、バックエンドの保存、Svelte・DOM・WebKit の描画と GPU は測定に含みません。この測定はページ状態の更新だけを比較し、受信時の JSON、解析時のピークメモリや実機の白画面障害の解消を確認した結果ではありません。

### LIVE ページへの応答

`live/surface.rs` は総行数・直近 120 行・要約全体と既存の録音・更新・保存 metadata の表示用 projection を作り、転写全文と待機行の配列は IPC に含めません。旧表示用 RPC の `live_get_surface`・`live_peek_day_surface`・`live_start_surface` はこの完全な表示 object を維持し、現在のページは次節の白板表付きの RPC を使います。一行の本文は切り詰めず、待機中の範囲は `pending_from_line` で表します。従来の完全な記録 API は保持し、プレビューのキャッシュ読み取りと開始処理は新旧 API で同じ実装を使います。開始に失敗しても別の命令による再試行を追加しません。

状態取得は LIVE ロック中に snapshot と revision を取得し、末尾の行の共有所有権だけを残して完全な転写・待機索引を破棄します。応答のエンコード中に索引を保持しないため、その応答のために後続の字幕追加が完全な索引をコピーすることもありません。プレビューは既存の保存ゲートでキャッシュを読み、開始は既存の検証・復元・通知・生成 driver を使います。開始応答の表示用変換は通知前に行い、通知に続く字幕追加が応答の完全な索引をコピーすることを防ぎます。これらの読み取りと JSON エンコードは `live/response.rs` の背景処理で行い、エンコード時は LIVE と保存のロックを保持しません。

ページと API は同じ表示用型を使い、デモの変換も UI に依存しない共通モジュールを使います。`views/live/liveTranscript.ts` は表示用応答と完全な記録応答の両方を統合し、更新 revision・欠落行・保存段階・録音の切り替えの判定を保持します。完全な記録応答の待機範囲は元の待機配列に従います。転写イベント、保存済み記録と Markdown の形式は変えません。要約通知の沿用白板は後述する version 付き参照を使います。

`live_finish_surface` は完全記録の終了命令と同じ録音所有権・マイクの停止と末尾の待機・要約・ファイル保存・背景 TODO の処理を使います。完了応答は `LiveSurfaceSaveResult` として保存先・保存済みフラグ・TODO の状態と候補・表示用 snapshot・`summary_markdown` を返します。表示用の概要は従来の保存プレビューと同じ節の境界・ECMAScript の空白除去を使います。全文 Markdown はファイルへ書いた後に応答 worker で破棄し、保存処理が AI 用に取得した完全な索引も応答を待つ前に解放します。必要な全文は背景 TODO の所有値に残します。

旧表示用の終了命令は `live-surface-saved` のみを発行し、同じ JSON 本文を終了応答にも使います。LIVE と tray はこの旧イベントと次節の白板表付きの保存イベントを購読するため、通常の保存で全文のイベントを解析しません。完全記録の終了命令は従来の完全な `live-session-saved` と応答を保持し、旧表示用イベントも発行して他の呼び出し元による保存を現在のページへ通知します。字幕のない終了は従来どおり保存イベントを発行しません。失敗時に別の終了命令を再試行する処理は追加しません。

回帰テストは空・119/120/121 行・一万行・長い一行を含む応答の全項目と共有テキストの所有権を検証します。エンコードを停止した実際の応答 worker の間に別の async task、字幕追加と録音の置換を進め、取得済み応答の不変性、ロックの解放、完全な索引と表示外の行を保持しないことを確認します。プレビューの保存ゲートとキャッシュの全項目、フロントエンドの新しい命令・エラー・デモの完全な保存、旧応答との統合結果も検証します。保存は実際の Markdown 構築と応答 worker で全項目・終了した録音 ID・TODO 候補・フラグ、旧命令の全文応答、ページ用のイベントだけを発行すること、保存イベントと応答の JSON 本文の一致を比較します。Rust と JavaScript は同じ概要 fixture で日本語・絵文字・CRLF・見出しの優先順位・FEFF と NEL の扱いを確認します。

2026-10-08 の合成入力で転写一万行をすべて待機行にも含め、実際の背景処理が返した JSON を比較すると、完全な状態応答は 1,598,624 bytes、表示用応答は 10,475 bytes でした。同じ一万行の保存 fixture で完全な Markdown を構築し、完全な保存応答は 2,268,634 bytes、表示用保存応答は 10,799 bytes でした。全要約と末尾 120 行の全文、保存先と TODO の全項目、従来の表示概要を保ったそれぞれの fixture の転送量比較です。キャッシュは引き続き背景処理で全体を読み取り・解析し、完全記録用 API は完全な応答を返します。実機 WebView の解析時間、アプリ全体の RSS、GPU と白画面障害の改善を計測した結果ではありません。

### LIVE ページの白板表の転送

ページの取得・講義 cache の preview・開始・終了は、それぞれ `live_get_surface_compact`・`live_peek_day_surface_compact`・`live_start_surface_compact`・`live_finish_surface_compact` を使います。`live/surface/compact.rs` の `CompactSurfaceSnapshot` は同じ表示用 projection を保持し、reply ごとの `whiteboard_table_version: 1`、完全な白板 object の `whiteboards` 配列、各区間の省略可能な `whiteboard_ref` を送ります。表の番号はこの reply 内だけに有効で、ネイティブの pointer や永続 ID は送信しません。全区間のタイトル・本文・用語・文字数、ライフサイクルの版・保存段階と末尾 120 行は保持します。

共有 Arc の pointer 索引で同じ版の内容を繰り返し hash せず、別の Arc として復元された過去 cache の白板も完全な内容の Hash/Eq で判定します。タイトル・layout・schema・正規化 marker、全ノードと全フィールド、辺とその向き・label・配列順が同じ場合だけ一つの表項目にします。hash 値だけで採用せず、衝突時も完全な等価比較を行います。未知の差を近似して除外する処理や、ノード数・履歴数の上限は追加しません。内容索引は借用、表の項目は Arc の参照であり、元の記録を変更したり全白板を複製したりしません。表の作成と JSON は既存の blocking worker で行い、LIVE・保存ロックの外で処理します。内容比較は独立した版の全内容を走査する費用が加わるため、この変更の全入力の CPU 高速化は主張しません。

`src/lib/liveBoardTransport.ts` は表の version・object・整数参照と範囲を確認し、各区間を従来の `LiveSummaryChunk` の形へ戻します。各参照に同じ白板 object を割り当て、沿用した区間の切り替えで WeakMap のレイアウトを再使用できます。表や参照フィールドを表示状態へ残さず、返答を跨いだ常駐 cache は作りません。decoder 単体では別の返答の白板は別の object です。表示状態への通知・復旧の適用時の同一性の再使用は次節で扱います。旧 `live_get_surface` などの表示用 RPC、完全な記録 API、日次 JSON cache と保存 Markdown は従来の完全な object のままです。

新しい終了経路は `live-surface-compact-saved` に reply と同じ JSON を一回発行します。従来の終了経路と `live-surface-saved` / `live-session-saved` の形式は維持します。LIVE は両方の表示用保存通知を所有して解除し、compact 通知の不正な表はログと状態再取得で処理します。托盤の状態更新も新しい通知を所有・合流し、古い登録や停止後の callback を無視します。開始・終了の失敗時に別コマンドで操作を再試行しません。demo は従来の完全な状態からの projection を使います。

`live/surface/compact/tests.rs` の五項目は、128 組の共有・独立した等値・空・存在しない白板を含む履歴で、還元して従来 DTO に解析した JSON の全 byte を比較します。タイトル・layout・schema・marker、ノードの十フィールド、辺の三フィールドと順序を個別に変えた場合の分離を検証します。非表示の字幕索引・行を保持しないこと、最後の所有者の解放、完全な出力、worker の保存 reply と一回の同じ通知、空の保存で通知しないことも確認します。既存の旧 RPC・保存形式の回帰も維持します。

`tests/live-board-transport.test.mjs` の三項目は 32 組の Unicode 履歴の全フィールド、共有 object の同一性、別 reply の独立性、保存 metadata、不正な表・数値・範囲と入力の不変性を確認します。実際の Tauri invoke wrapper の既存十一項目は新コマンド・記録 ID と失敗時の単一要求を使い、demo と完全記録の動作も維持します。托盤の既存四項目は新通知を含む登録・解除と古い callback を検証します。

2026-10-08 の合成した保存 fixture では、一万行の原文と 32 区間が同じ 96 ノードの完全白板を沿用する場合、実際の worker が返した旧表示 JSON は 2904637 bytes、新表示 JSON は 105233 bytes でした。全区間・用語・出典・ノード・辺、保存先・TODO と表示用の完全な末尾 120 行・保存要約を同じ内容へ還元しています。これは転送 JSON の byte 数であり、実際の WebView の解析時間・RSS・GPU の改善率へ換算しません。完全な原文と Markdown の保存は保持します。

`node scripts/check-live-whiteboard-browser.mjs` のブラウザー検証は実際の LIVE 白板状態・ページ、Markdown 白板、レイアウトと今回の decoder を使います。32 区間を切り替える四項目で、同じ完全白板 object、両 renderer のレイアウトの再使用、DOM ノードの保持、全フィールドと実測 viewport の保持を確認しました。後述の通知・復旧・version 付き参照と既存の重複 ID・prototype 名・topic・字幕更新・解除を含む 160 項目を確認します。ネイティブ IPC は fixture で、インストール済みアプリやマイクは起動しません。GPU の白画面を再現・解消した検証ではありません。

### LIVE 通知と復旧を跨ぐ白板の同一性

通常の `live-session-updated` は最新区間の完全な白板を送るため、ネイティブの同じ Arc を沿用しても JSON を解析した白板は別 object になります。`liveBoardIdentity.ts` は、同じ録音内の一件の正常な append で、最後の非 null 白板と全内容が等しい場合だけ既存の object を割り当てます。新しい区間のタイトル・範囲・本文・用語などは受信した内容を保持します。通知を変更せず、白板参照の変更が必要な区間だけ浅く複製し、別の cache や白板の hash・JSON 文字列を常駐させません。録音の交換・不足した区間・古い版は既存の扱いを保ち、以前の録音の白板を再使用しません。

等価比較は既知の schema の一部に限定せず、未知の object フィールド、`__proto__` などの自前のキー、すべてのノード・辺・出典と配列の順序を確認します。object のキーの挿入順は等価条件に含めず、フィールドの欠落と null は区別します。空白板も最新の完全白板として扱います。64 段以上の深い未知の構造や非 JSON object は保守的に元の参照を保ち、内容を捨てません。この上限は保持する白板・ノード・履歴の上限ではありません。

`mergeLiveSnapshot` の復旧でより長い履歴を採用する際も、同じ録音の同じ区間位置の等値白板と、受信履歴の直前の非 null 白板を再使用します。区間 metadata は受信側を保持し、履歴配列は変更があるときだけ複製します。別の録音や初回の履歴では、その受信履歴内だけで沿用を共有します。字幕・要約の欠落の検知、版の watermark、保存段階、原文末尾と待機範囲は既存の更新・復旧処理を保ちます。

`tests/live-board-identity.test.mjs` の七項目は、変更直前の表示用更新を独立した `live-board-identity-before.ts` に固定して、768 組の通知・復旧の完全な JSON 値と入力の不変性を比較します。32 件の独立した通知から一つの完全白板を保持し、表示の到達可能な object graph に受信側の不要な白板・ノードがないこと、全フィールド・順序・未知のキーの変更、空白板・null・省略・録音交換、深い非 wire 入力を確認します。実際の layout と WeakMap wrapper で、overview と topic の二結果の同一性、変わった白板の再計算も検証します。旧実装では五項目が失敗し、完全値の互換と深い入力の二項目は通ります。以前の完全記録の対比 fixture は別ファイルで維持します。

手動測定は `node scripts/benchmark-live-board-identity.mjs` です。新旧の実際の更新、同じ layout engine と wrapper を使い、32 件の独立した JSON 解析済み通知を一組として三回の warmup 後、九回の交互実行の中央値を測ります。両処理の完全な表示状態と全 layout 値が一致します。通知の準備・JSON 解析・結果の比較と破棄・DOM/WebKit・IPC・アプリの RSS と GPU は除外します。異なる白板の JSON byte の合計は表現するデータの量で、JS heap の測定ではありません。

2026-10-08 の macOS / Node で、24 ノード・48 辺を 32 段沿用する更新と二 layout の合計は 34.587500 → 1.627667 ms、96 ノード・192 辺では 741.338208 → 24.748084 ms でした。両条件で実際の layout 計算は 64 → 2 回、白板 object は 32 → 1 個です。96 ノードでは表示状態の異なる白板の JSON 合計が 1271936 → 39748 bytes になります。一方、比較を含む純粋な更新だけは 24 ノードで 0.024042 → 0.400375 ms、96 ノードで 0.026666 → 1.522042 ms と増えます。

八段ごとに白板が変わる 96 ノードの条件では layout が 64 → 8 回、合計は 736.269792 → 94.950875 ms、純粋な更新は 0.026792 → 1.417667 ms でした。全段で内容が変わる条件は layout 64 回と白板 32 個を保持し、合計は 739.774000 → 738.552833 ms とほぼ同じ、純粋な更新は 0.027834 → 0.029792 ms です。この同一性処理だけでは全入力の高速化や IPC 転送量の減少を主張しません。実際の沿用通知の JSON は次節の参照で軽量化します。

ブラウザーの追加六項目は、独立して解析した 32 通知と復旧を実際の LIVE・Markdown 白板へ適用し、両 layout と DOM の再使用、手動 zoom・pan の保持、既存の区間変更時のノード選択解除、内容変更時の再計算、完全な復旧 JSON を確認します。GPU の白画面についての原因・解消の証明とは分けて扱います。

### LIVE 通知の沿用白板の参照

`live/notification.rs` は同じ `live-session-updated` を一回だけ発行する version 付きの wire を構成します。`whiteboard_delta_version: 1` を付け、新しい要約の白板が直前の非 null 白板と同じ Arc の場合だけ、その区間の zero-based index を `latest_summary.whiteboard_from_summary` に送ります。この場合だけ inline の `whiteboard` を省略します。新しい白板・独立した等値の白板・最初の白板は完全な内容を送り、白板のない段は従来どおり省略します。空白板も一つの完全な版として扱います。pointer や永続 ID は wire へ送らず、参照はその録音の append-only な区間配列にだけ有効です。

`support::capture_update` は同じ LIVE ロック中に metadata、最新要約の Arc と参照の位置を取得します。直前の非 null 白板だけを探し `Arc::ptr_eq` を使うため、全白板の hash・内容比較やノードの走査をロック内で行いません。通知は最新要約一件だけを所有し、字幕・待機行・全要約の索引・以前の区間を保持しません。ロックを解放してから既存の emit 経路で encode します。metadata は generic `LiveSessionUpdate<Chunk>` へ移動して schema を共用し、文字列や用語・白板をさらに深く clone しません。追加の完全通知は並行して送りません。イベントの状態を読む tray とネイティブ字幕 overlay は同じイベント名・活動・版・録音 ID を使います。完全な RPC、日次 cache と Markdown の形式は保持します。

`views/live/liveNotification.ts` は旧来の完全通知も受け取り、version 1 では参照・整数・範囲・inline 白板との矛盾を確認します。version のない参照や未知の version は復旧を要求します。新しい録音の同じ index を古い録音の白板へ解決せず、同じ録音で一段だけを追加できる場合に、既存の完全白板を割り当てます。参照先や途中の段が欠ける場合は、既存の件数・版の処理で metadata を適用して完全なページを再取得し、解決できない白板を持つ段を挿入しません。古い通知は復旧の要求もせず無視します。重複通知、古い保存段階、旧 snapshot と復旧の競合は既存の処理へ委譲します。wire の version・参照を表示状態へ残しません。

Rust の追加四項目は、実際の capture から 256 組の共有・新規・独立等値・空・不存在の白板を還元し、旧完全更新の全フィールドを比較します。別々の capture の版だけは、大小関係を確認した上で同じ値へ揃えます。native overlay の小さい型で lifecycle を読めること、通知の clone と encode が LIVE ロック・旧区間・字幕索引を保持しないこと、最後の所有者の解放も確認します。八組の実際の native wire を `tests/fixtures/live-notification-wire.json` に保存し、JavaScript の六項目で同じ完全状態への還元、768 組の連続更新、欠落・古い復旧・録音交換・重複・不正な参照と旧通知を検証します。fixture の更新は必要なときだけ `SELAH_NOTIFICATION_FIXTURE_PATH` を明示して該当 Rust テストを実行します。

2026-10-08 の合成したネイティブ白板で実際の通知を JSON encode すると、沿用時の payload は三ノードで 3670 → 749 bytes、96 ノードで 91016 → 749 bytes、512 ノードで 484264 → 749 bytes、4096 ノードで 3890269 → 749 bytes でした。全モデル出典・ノード・辺は既存の完全な区間へ解決して保持し、新規白板は省略しません。この例の通知には区間の本文・用語・metadata も含まれ、749 bytes は一般の本文長に対する固定値ではありません。Tauri の dispatch wrapper や IPC 全体、ネイティブの CPU 時間・RSS を計測した値ではありません。

`node scripts/benchmark-live-notification.mjs` は、完全通知と参照通知の二つの受信経路を同じ実際の更新処理・layout wrapper と engine で比較します。初期の完全白板と overview / topic は既に表示済みとして、三回の warmup 後の九回の交互 batch の中央値を測ります。32 件の各通知の `JSON.parse`、参照の検証または等価比較、状態更新と二つの実際の cache lookup を含みます。両経路とも layout の計算は初回の二回だけで、完全な表示状態と layout の全値が一致します。fixture の作成、初期白板の解析と layout、結果の比較・破棄、ネイティブの encode、IPC dispatch、DOM/WebKit、アプリの RSS と GPU は除外します。

同じ macOS / Node の最終ソースで、24 ノード・48 辺の 32 件は 1.062041 → 0.089667 ms、合成 JSON の合計は 336698 → 16368 bytes でした。96 ノード・192 辺では 3.463333 → 0.098542 ms、合計は 1287322 → 16368 bytes です。この JS fixture は前段の Rust fixture と本文・白板の長さが異なるため、Rust の byte 値と合わせて改善率へ換算しません。

ブラウザーの追加六項目は、32 件の version 付き通知を実際の LIVE と Markdown 白板へ適用し、完全な白板参照・layout と DOM の再使用、表示からの wire フィールドの除外、欠けた参照時の復旧要求、完全な復旧の JSON、遅れた参照の無視を確認します。既存の確認を含む 160 項目、ブラウザーの error / warn がないことを確認します。インストール済みアプリやマイクを起動しない局所検証であり、GPU の白画面の再現・解消は未証明です。

### LIVE の白板出典補完の借用

`live/whiteboard/excerpts.rs` は、変更の必要がない転写行とノードの照合語を `Cow<str>` で借用します。原文の Unicode scalar 数、正規化後の UTF-8 byte 数と空白・ASCII 大文字の有無を同じ走査で取得し、変わる文字列だけに一つの必要容量のバッファを作ります。空白除去の String をさらに小文字化のために複製しません。Unicode の空白と ASCII のみの小文字化という既存の比較を保ち、Unicode 全体の case folding は導入しません。すべて空白の場合も原文の長さに比例するバッファを予約しません。

照合語はノードの label・detail の分割片を借用し、必要な ASCII 変換だけを所有します。文字数を各語に保持し、安定した長さ順の並べ替え・従来の隣接項目だけの重複除外・最大八語を保ちます。得点の計算用にもう一つ Vec を作らず、八項目の小さい stack 配列に照合語への参照と得点を置きます。Cow の借用・所有の分岐をノードごとに一度解決し、各行との照合のために文字数を数え直しません。行ごとの候補検索は既存と同じ全行の走査で、近似検索や候補・本文の削減は追加しません。

上一版の ID・label の索引は、供給済みでない講義出典のノードを初めて処理する際だけ作り、二つの map を一回の走査で作成します。後から現れる非空の重複 ID・label、ID の優先、最初に一致する用語出典、転写得点・原文文字数・最後の同点行の順序を保持します。継承・用語・転写から補完する引用は trim 後の八十 Unicode scalar と必要な省略記号を一回の分配で作り、長い原文の全文字数を引用のために走査しません。供給済みの引用は従来どおりそのまま保持します。入力の全行・ノードを変更せず、所有する最終白板だけを返し、借用した語・索引はこの更新内だけで解放します。

`excerpts/tests.rs` の四項目は、変更直前の索引実装を `excerpts/before.rs` に凍結し、1536 組の完全な白板 JSON と転写・過去白板・用語の不変性を比較します。Unicode・空・自前の出典・継承・用語・転写・外部出典と重複 ID を含みます。固定期待値で変更不要の借用参照、空白の全削除と ASCII/Unicode の違い、安定順序と八語、同じ長さの非隣接重複語、八十文字の前後、長い引用と最後の非空の ID の優先を確認します。以前の照合・同点・出典優先の三項目と実際の背景生成のテストも通します。凍結処理と fixture は製品ビルドへ含めません。

手動比較は `cargo run --manifest-path src-tauri/Cargo.toml --example benchmark-live-board-excerpts` です。実際の新旧の補完処理を生成した所有白板で動かし、三回の warmup 後の九回の交互 batch の中央値を測ります。分配の測定は別の呼び出しで行い、最終 JSON の全 byte が一致します。両処理に所有入力を渡すためのモデル白板の clone、継承索引・語・必要な字幕索引・候補照合と引用の作成を含み、返された完全な所有白板も分配量に含みます。fixture の準備、JSON 比較・出力破棄、モデル JSON の解析・モデル・IPC・アプリの UI/RSS/GPU と allocator 管理領域・realloc 内部は除外します。入力の借用で正規化の分配がなくなる場合も、元の入力白板・転写のメモリーがなくなる意味ではありません。全引用が供給済みの条件では、新処理の分配回数・総量・ピーク・返却量が入力白板の clone だけと一致することも検証します。

2026-10-08 の macOS で、既存の互換 `serde` / `serde_json` rlib と dependency directory を渡し `rustc --edition=2021 -O` で単独に最適化した最終ソースの測定では、75 ノード・500 行が 2.178084 → 2.106167 ms、5000 行が 23.986583 → 23.320833 ms でした。分配要求はそれぞれ 6079 → 1512 回 / 34214 → 5775 回、総要求量は 424756 → 201780 bytes / 2970196 → 1035254 bytes です。5000 行の同時生存する要求量のピークは 974384 → 969618 bytes とほぼ同じで、総分配量の減少を同じ割合の常駐メモリー減少と解釈しません。既に正規化済みの 5000 行の別条件では、32807 → 1039 回、ピーク 619516 → 275510 bytes、20.528292 → 19.542291 ms でした。500 行の出力 36642 bytes と、5000 行の二条件の出力 36492 / 27792 bytes はそれぞれ全 byte が一致します。

512 ノードに全引用を供給済みで、4096 ノードの前版がある条件は 0.140208 → 0.058625 ms、追加の継承索引を作らないためピーク 326221 → 224829 bytes でした。512 ノードで長い引用を継承する別条件は 0.323666 → 0.132875 ms、返された所有白板の要求量 284687 → 271375 bytes です。後者は合成した長い既存出典の条件であり、通常のモデル解析で制限された出典長と同一条件には扱いません。空の白板では分配と時間は同じ、小さい八ノード・24 行では 0.028433 → 0.021550 ms でした。

通常の最適化なし dev profile の同じ最終ソースでは、75 ノード・500 行は 60.550958 → 61.375292 ms、5000 行は 938.589583 → 653.170000 ms、既に正規化済みの 5000 行は 852.116750 → 803.613833 ms、供給済みは 2.176542 → 0.139708 ms、長い継承は 2.164500 → 2.639250 ms、八ノードは 0.433966 → 0.454596 ms でした。調整前の内側の Cow 分岐はさらに遅い場合があったため、参照と得点の小さい stack 配列へ集約しています。分配量と出力の一致は dev と最適化後で同じです。速度が少し低下する条件もあり、全入力・全 build profile の高速化は主張しません。これらは合成した区間の局所測定で、アプリ全体の release build・RSS・GPU と LIVE 白画面の解消を計測した結果ではありません。

### LIVE の完了した白板の共有

`LiveSummaryChunk.whiteboard` は `Option<SharedWhiteboard>` (`Arc<LiveWhiteboard>`) です。モデルが白板を省略した場合、または既存の異常収縮・主ノード ID・構造 ID・横断辺の保護条件で前の白板を採用する場合、`live/ai_output/reconcile.rs` は同じ不変の版を参照します。区間ごとに全ノード・辺・本文を深く複製する処理を除去します。新しいモデル白板は従来どおり所有値で解析し、出典を補完した後に一度だけ Arc に移します。新しい版の採用では Arc 自体の分配が一回増えますが、既存の Vec・String のバッファは移動し、旧版を変更しません。共有は保持済みの要約・snapshot の寿命に従い、別の cache や恒久的な参照を追加しません。

最新白板は末尾から最初の `Some` を選び、空白板も有効な最新版として保持します。全ての拒否条件はノード数が増えない場合だけに適用されるため、増加したモデル出力では ID・横断辺の索引を作る前に従来と同じ採用結果を返します。JSON の cache・応答・通知には引き続き各区間の完全な白板 object を出力し、共有 ID を導入しません。Markdown とモデル本文の内容も変えません。既存 cache の同一 object が複数回出現する場合、復元時は別々の Arc に解析され、読み込んだ白板の重複排除は行いません。

`live/ai_output/reconcile/tests.rs` の五項目は、この変更の直前に凍結した所有値の処理と 3136 組の入力で完全な JSON byte を比較します。省略・空・Unicode・重複 ID・増加・収縮・ID の置換・辺の除去を含み、各保護条件も固定期待値で確認します。512 ノードの同じ白板を 64 区間で沿用する場合の Arc・ノード・辺・本文の同一性、保持済み snapshot の不変性、最後の参照がなくなった後の解放、新しいモデルのバッファの移動、完全な JSON 往復と空の最新版を確認します。実際の背景生成処理のテストでも省略・不正な応答・異常収縮で前の版の Arc が保持されることを検証します。凍結関数と fixture は製品ビルドへ含めません。

手動比較は `cargo run --manifest-path src-tauri/Cargo.toml --example benchmark-live-board-retention` です。実際の型と新旧の処理を合成白板で動かし、三回の warmup 後の九回の交互 batch の中央値を測ります。前版沿用の既存診断出力、白板だけの保持配列、または借用した保護条件の判定を測り、入力白板の作成・解析・出典補完、JSON 比較・出力破棄、完全な要約の metadata、モデル・IPC・UI・RSS・GPU と allocator 管理領域は除外します。System allocator の分配計数は時間測定と別に行います。参照先の入力白板は既に存在するため、沿用で新しい分配がゼロでも白板自体のメモリーがゼロになる意味ではありません。

2026-10-08 の macOS で既存の互換 `serde` / `serde_json` rlib と dependency directory により `rustc --edition=2021 -O` で独立して最適化した測定では、512 / 4096 ノードの沿用が 0.098394 → 0.001635 ms / 0.781983 → 0.002837 ms、追加の分配要求は 6536 → 0 回 / 52232 → 0 回、要求量は 536765 → 0 bytes / 4315506 → 0 bytes でした。512 ノードの白板を 32 回保持する配列では 3.200111 → 0.047139 ms、209153 → 1 回、同時生存する追加要求量は 17180576 → 256 bytes でした。新しい 256 bytes は参照配列だけで、共通白板の既存分配と完全な要約の本文は含みません。4096 → 4097 ノードの借用した採用判定は 0.643916 → 0.000016 ms、44 → 0 回でした。同数・横断辺の除去・極端な収縮の判定は同じ経路であり、全条件の高速化を主張しません。

通常の最適化なし dev profile では、512 / 4096 ノードの沿用が 0.142183 → 0.001440 ms / 1.129537 → 0.001704 ms、32 回の保持が 4.488124 → 0.046930 ms、増加の判定が 8.453708 → 0.000025 ms でした。分配量と全出力の一致は最適化後と同じです。これらは新しい白板の解析・採用全体やアプリ全体の release build の測定ではなく、実機 GPU と LIVE 白画面の解消を証明しません。

### LIVE の借用した要求本文

`live/context_text.rs` の `ContextPart` は、文字列・曜日/時限の整数・取得済みの転写行・要約を借用した片です。各片の UTF-8 byte 数を先に求め、`build_context_text` が一つの最終 String へ直接追加します。転写や全要約を一時 String にした後で要求本文へコピーする処理を除去します。整数は ASCII の桁数と符号を容量に含め、`i32::MIN` の絶対値も unsigned で扱います。文字列の byte 長だけを読み、本文を長さの計算のために走査しません。片は小さい stack 上の配列と借用 slice で、Arc の各行・要約を複製したり、表示や cache へ保持したりしません。

`live/generation/request_text.rs` は最初の分割要約・全体要約・TODO の user 本文に共通の片を使います。テンプレートの文面、自由ノートと講義の違い、教員の既定値、番号と改行を保持します。最初の要約は転写全文と直前二区間、全体要約と TODO は全分割要約の全文、それぞれ終盤 24/80 行を使います。空の直前要約は従来どおり `なし`、空の全体要約の素材は空文字列です。白板の `WhiteboardContext` も同じ転写の片を使い、既存の末尾 500 行と省略行数の注記を最終本文へ直接追加します。`requests.rs` の blocking worker、取得済みの共有記録、設定・system prompt・モデル呼び出し・保存値は同じままです。

`context_text/tests.rs` の二項目は、空・Unicode・改行・空白の片、追記、0/9/10/99/100 と最大 usize の省略行数、整数の符号と境界で出力 byte と予約容量を比較します。`generation/request_text/tests.rs` の三項目は、この変更の直前に凍結した組み立てと 1536 組の user 本文を比較します。転写数 0/1/24/25/80/81/500/501、要約数 0/1/2/3/12/128、講義・自由ノート、空白・Unicode・空教員・整数を含み、入力 JSON の不変性と各 Arc の所有数も確認します。3000 行・140 要約で全文と既存の選別を固定期待値で検証し、長い本文は入力解放後も完全に保持します。白板履歴の既存 864 組の比較と、実際の要求の四言語・各転写境界の全メッセージ比較も通します。凍結関数・fixture は製品ビルドには入りません。

`cargo run --manifest-path src-tauri/Cargo.toml --example benchmark-live-request-text` は実際の本文ソースと直前の凍結関数を生成入力で比較します。三回の warmup 後の九回の交互 batch の中央値を使い、別の呼び出しで System allocator の要求数・総量・同時生存量を計数します。全 user 本文の byte 一致と、どの測定ケースでも最終本文を一回の分配で作ることを確認します。全体要約と TODO の授業情報・注記も含み、最初の要約の既に整形済みの授業情報・注記と TODO の計画は入力です。fixture 作成、出力破棄・比較、設定・system prompt・AI/network・async/IPC・アプリ UI/RSS/GPU・allocator 管理領域と realloc 内部は含みません。アプリ・マイク・ユーザーの記録を起動しません。

2026-10-08 の macOS の実際のソースを既存の互換 `serde` rlib と dependency directory で `rustc --edition=2021 -O` により独立して最適化した比較では、一万行・64 要約の最初の user 本文 10,020,613 bytes が 7.217528 → 2.540791 ms、分配要求 22 → 1 回、総要求量 62,217,443 → 10,020,613 bytes、同時生存量のピーク 36,116,648 → 10,020,613 bytes でした。1000 行・512 要約の全体要約本文 721,410 bytes は 0.128373 → 0.036878 ms、25 → 1 回、総量 4,899,562 → 721,410 bytes、ピーク 2,799,014 → 721,410 bytes、TODO 本文 777,357 bytes は 0.105743 → 0.039618 ms、28 → 1 回、総量 5,094,323 → 777,357 bytes、ピーク 2,896,546 → 777,357 bytes でした。これはアプリ全体の release build の測定ではありません。出力の省略による差ではなく、分配量を製品 RSS や実機の GPU 白画面障害の改善へ換算しません。

通常の最適化なし dev profile では、同じ一万行の最初の本文が 5.362319 → 3.332764 ms、1000 行・512 要約の全体要約が 0.255988 → 0.170401 ms、TODO が 0.258211 → 0.174391 ms でした。分配の計数と全文の一致は最適化後と同じです。空の最小入力では、byte 数の計算と片の dispatch が加わり、最初の本文 0.000348 → 0.000783 ms、全体要約 0.000260 → 0.000766 ms、TODO 0.000388 → 0.001871 ms と少し遅くなります。全入力での高速化を主張せず、長い転写・全要約の再コピーと大きい一時領域を除くことが狙いです。

### LIVE 白板への要約履歴の組み立て

`live/whiteboard/history.rs` は、白板の二段目の user 本文を借用した入力から組み立てます。要約・用語の各行を一時 String にし、その後に完全履歴 String と今回の区間 String を作り、最後に要求本文へ再コピーする処理を除去します。`WhiteboardContext` は UTF-8 の出力 byte 数を先に求め、一つの最終 String に必要な容量を確保して本文・区切り・指示を直接追加します。容量の計算は既存文字列の byte 長と区間番号の桁数を読み、本文を文字ごとに走査しません。最近の要約を使う最初の要求も、上記の借用した片で要約を最終本文へ直接追加します。

これは `live/generation/requests.rs` の既存の blocking worker 内で実行します。取得済み Arc の要約・用語は借用するだけで、履歴の深い複製や別の長期 cache は作りません。最終要求は所有 String なので、生成準備後に入力の共有所有権を解放しても使用できます。全要約の本文、古い区間の用語名、最新四区間の用語説明・外部出典、今回の区間、元の空白・改行・区間番号・指示を保持します。空の context の `なし`、最近の要約の件数、補助転写の既存の末尾 500 行、その他のモデル要求・保存値の規則は変更しません。

`live/whiteboard/history/tests.rs` の四項目は、凍結した旧整形と最終本文の組み立てを比較します。864 組の生成履歴で全文 byte、入力の不変性、最近の要約の zero・有限・最大 limit、Unicode・空白と用語を確認します。固定期待値で最新四区間の境界と全文・出典を検証し、9/10・99/100・999/1000 区間の番号も確認します。64 区間の長い全文と用語は入力解放後も保持します。出力 byte 数と予約容量の一致はこれらのケースで確認し、単一の分配要求は独立した比較プログラムでも計数します。凍結関数と fixture は製品ビルドに含めません。既存の実際の要求の回帰テストも、四言語、講義・自由ノート、転写数の境界で system/user の全メッセージを比較します。

手動比較は `cargo run --manifest-path src-tauri/Cargo.toml --example benchmark-live-board-history` です。実際の型と組み立てソースを生成データに使い、三回の warmup 後の九回の交互 batch の中央値を測ります。分配は別の呼び出しで System allocator への要求回数・総量・同時生存量を計数し、最終 String の全 byte の一致と一回の分配を確認します。履歴・今回の区間と最終 user 本文の組み立てを含み、fixture の作成、出力破棄・比較、既に整形済みの授業情報・累積白板・補助転写の作成、async/IPC・モデル・network・アプリの UI/RSS/GPU・allocator 管理領域と realloc 内部は測定しません。アプリ・マイク・モデル・ユーザーのファイルを起動しません。

2026-10-08 の macOS の実際のソースを既存の互換 `serde` rlib と dependency directory で `rustc --edition=2021 -O` により独立して最適化した比較では、8 区間・出力 11,979 bytes が 0.005805 → 0.000634 ms、64 区間・85,397 bytes が 0.020748 → 0.003292 ms、512 区間・674,870 bytes が 0.167058 → 0.033476 ms でした。これはアプリ全体の release build の計測ではありません。512 区間では分配要求が 3,210 → 1 回、総要求量が 5,392,285 → 674,870 bytes、同時生存量のピークが 2,643,678 → 674,870 bytes でした。64 区間の長い本文の出力 12,855,189 bytes の場合は 1.626777 → 0.308680 ms、総要求量 102,634,941 → 12,855,189 bytes、同時生存量のピーク 51,410,342 → 12,855,189 bytes でした。どの入力も出力全 byte が一致し、出力の削減や履歴の打ち切りによる差ではありません。これらの局所的な分配量を製品全体の RSS の改善や実機 GPU の白画面の解消へ換算しません。

通常の最適化なし dev profile でも、同じ 8/64/512 区間が 0.014777 → 0.009280 ms、0.093370 → 0.054267 ms、0.786895 → 0.462054 ms、長い本文が 3.193944 → 0.679430 ms でした。履歴が空の小さい 865 bytes の出力だけは 0.000643 → 0.000906 ms と少し遅く、容量の計算費用も含むため全入力での高速化は主張しません。分配の計数と全出力の一致は dev と最適化後で同じです。

### LIVE の累積白板の入力整形

次の白板生成に渡す `live/ai_output/context.rs` は、最新の白板から親ごとの分岐・用語と ID の借用索引を一回作り、主ノードごとに全ノードを再検索しません。索引は一回の整形中だけ保持し、履歴やノード本文を複製・cache しません。主ノードとその分岐、直下の用語、孤立ノード、横断辺を従来の順序で同じ出力 String へ追加します。行ごとの `format!`、用語の join、辺ごとの String と辺全体の join を除去します。

プロンプトの内容と既存の表示規則は保持します。主ノードの detail は六十、分岐・孤立ノードは四十八 Unicode scalar の既存の境界を使い、UTF-8 の借用 prefix と必要な省略記号を直接追加します。空白だけの detail の区切り、重複 ID の最後のノードによる辺判定と ID による孤立項目の抑制、重複主ノードの分岐再掲、親子・用語・不明端点の辺の除外、最新の空白板の採用も変えません。本文・項目数を追加で制限せず、同じ内容を組み立てる際の再検索と一時文字列を減らす変更です。

`live/ai_output/context/tests.rs` は凍結した従来関数 `context/before.rs` と七百六十八組の生成入力の全出力 byte、入力の不変性、Unicode の境界、順序と辺の除外の固定期待値、空・存在しない白板、四千九十六ノード全件の出力を比較します。凍結関数と fixture は製品ビルドへ含めません。手動比較は `cargo run --manifest-path src-tauri/Cargo.toml --example benchmark-live-board-context` で、実際の型・整形ソースと従来関数を生成データだけで実行します。アプリ・マイク・AI・ユーザーのファイル・保存・IPC は起動しません。三回の warmup 後に九回の交互 batch の中央値を一回あたりで表示し、別呼び出しで分配要求数・総量・同時生存量を計測します。出力 String を含む局所測定で、fixture 準備・出力破棄・比較、allocator 管理領域と realloc 内部、モデルの待機、製品全体の CPU/RSS・GPU と LIVE の白画面は測定しません。

2026-10-08 の macOS の局所比較では、通常の最適化なし dev profile で十六ノードは 0.080679 → 0.100658 ms、九十六ノードは 0.567519 → 0.581852 ms と少し遅く、五百十二ノードは 4.492662 → 2.998849 ms、四千九十六ノードは 143.593754 → 26.418866 ms でした。製品と同じソースを `rustc --edition=2021 -O` で独立して最適化した場合は、それぞれ 0.015682 → 0.009000 ms、0.101443 → 0.049776 ms、0.888579 → 0.293833 ms、23.710154 → 3.373562 ms でした。独立コンパイルには既に作成された `serde` の derive / rc 対応 rlib と dependency directory を渡し、アプリ全体の release build の測定とは扱いません。

両条件の分配要求数は九十六ノードで 648 → 79、四千九十六ノードで 26,673 → 2,107、同時生存する要求量のピークは 31,201 → 24,672 bytes と 1,260,916 → 999,456 bytes でした。出力はそれぞれ 9,299 / 422,869 bytes で全 byte が一致します。小さい入力では分岐索引の作成費用が加わり、最適化なしの小白板の速度改善は主張しません。大きい入力の反復検索と一時文字列を減らすことが狙いで、ノード数は生成 fixture の条件です。

### LIVE の白板 JSON の所有値への変換

`live/ai_output/board.rs` はモデルの JSON の文字列フィールドを借用し、数値だけ従来どおり文字列へ変換します。boolean・null・配列・object は従来の空フィールドとして扱います。ノードとタイトルの長さ制限は Unicode scalar の境界で借用 prefix を求め、最終フィールドに必要な容量だけを予約します。まず全文を複製してから文字数を数え直し、別の String に切り詰める処理を除去します。長さ制限・trim・省略記号は既存の規則のままで、ノード数と白板の内容へ新しい上限を追加しません。

layout・kind・node type・role・source type は既存の ASCII 大文字小文字と別名・既定値を比較し、正規名の静的文字列を返します。出力ノードが所有する String は一回だけ作り、判定用の lowercase String と node type の複製を作りません。親の修復・欠落した主ノードの補完・通常の ID の生成と接尾辞は従来の処理を保持します。重複 ID に付けた接尾辞がモデルの既存 ID と再び衝突する場合だけ、さらに数値の接尾辞を試し、未使用の ID を選びます。以前はこの場合も重複 ID が残り、白板の keyed node と ID map に衝突を渡していました。既存の ID と端点の参照を変えず、重複した後続ノードへ新しい ID を割り当て、ノードを除去しません。この修正は GPU 障害の再現・解消を確認した結果ではありません。

辺はノードの ID map で存在を判定し、別の既知 ID set を作りません。入力端点は借用して検証し、構造辺の対と用語辺の去重には最終ノードの ID を借用します。無効・重複辺の端点や去重用 ID の String を複製せず、採用する辺だけに所有文字列を渡します。数値端点の一時文字列を去重 set に残さず、検証済みノードの ID を使います。構造辺の最初の向きと label、用語辺の親から用語への向き、逆向き重複の除外、用語の優先順位と自動追加の順序は保持します。出力全体は引き続き所有値で、入力 JSON の破棄後にも使用できます。

`live/ai_output/board/tests.rs` の七項目は凍結した `board/before.rs` と六百七十二組の生成入力の出力 JSON 全 byte と入力の不変性を比較します。固定期待値で別名・親の修復・用語辺の優先、数字・無効フィールド、既存の Unicode 境界を検証し、四千九十六ノード全件と入力破棄後の所有値も確認します。ID の接尾辞衝突の二項目は修正前の実関数で失敗し、単一・連続衝突でも全ノードを異なる ID で保持し、既存の ID と辺の端点・親の参照を保持することを検証します。衝突した入力の出力だけは上記の修正に従い、従来の重複 ID を期待値にしません。凍結関数と fixture は製品ビルドに含めません。

手動測定は `cargo run --manifest-path src-tauri/Cargo.toml --example benchmark-live-board-parser` です。実際の型と解析ソースを生成 JSON に使い、三回の warmup 後、九回の交互 batch の中央値を一回あたりで比較します。分配要求数・総量・同時生存量と最終所有値は別呼び出しで計測し、出力全 JSON の byte 一致を確認します。これは JSON Value が既に存在する時点からのフィールド変換・正規化・辺の去重だけの比較です。入力の JSON 文字列の decode と fixture 作成、出力破棄・一致比較、allocator の管理領域と realloc 内部、モデル・ネットワーク、製品の UI・RSS・GPU と LIVE の白画面を計測しません。

2026-10-08 の macOS の生成入力では、九十六ノード・四十五辺の出力 74,993 bytes が一致し、分配要求数は 3,506 → 1,338、総要求量は 274,814 → 147,570 bytes、同時生存量のピークは 119,872 → 106,518 bytes でした。四千九十六ノード・二千四十七辺は出力 3,255,156 bytes が一致し、148,239 → 55,965 回、11,499,704 → 5,847,200 bytes、4,970,705 → 4,338,692 bytes でした。最終所有値の要求量も 104,654 → 94,576 bytes と 4,138,295 → 3,703,689 bytes になり、ノード・辺と出力内容は保持します。

最適化なしの dev profile の時間は十六ノード 0.397172 → 0.487729 ms、九十六ノード 3.155330 → 3.351432 ms、五百十二ノード 16.770542 → 17.939500 ms、四千九十六ノード 157.800555 → 165.248777 ms と少し遅く、分配の削減をそのまま速度改善と扱いません。最終ソースを既存の互換 `serde` / `serde_json` rlib と dependency directory で `rustc --edition=2021 -O` により独立して最適化した比較は、それぞれ 0.091952 → 0.077126 ms、0.651724 → 0.551795 ms、3.920110 → 3.286125 ms、36.637847 → 32.413555 ms でした。これはアプリ全体の release build の測定ではありません。

十六ノードの label・detail・source excerpt・external source に長い日本語・絵文字の文字列を渡す条件では、最適化なし 2.245166 → 0.889222 ms、最適化後 0.929638 → 0.084680 ms、総要求量は 6,712,493 → 29,564 bytes、同時生存量のピークは 237,415 → 25,370 bytes でした。既存の同じ長さ制限で出力 20,888 bytes が一致します。各分配量は比較プログラムの System allocator への要求の計数で、実際の RSS やモデルの JSON decode 中のピークへ換算しません。ID 衝突の修正は上記の例外入力の動作に限定し、この比較入力のノード ID は衝突しません。

### LIVE の授業計画読み取り

TODO の生成準備は `db/session_plans.rs` の `get_session_plans_for_course` で対象講義だけの授業計画を取得します。以前の全授業計画の読み取り・HashMap への分割・講義の検索を、既存の `(kgc_code, session_num)` の索引による読み取りへ置き換えます。対象講義の全行と全文を回数順に返し、提示する最初の 18 行、項目の文字数、シラバスの四項目、次回授業候補と生成言語は既存の規則を保持します。

入力と保存済みコードの Unicode の前後空白を扱う従来の一致も保持します。完全一致が見つからない場合だけ、既存のコード索引で重複のないコードを列挙し、Rust の `trim` で照合してから対象の行を読みます。この場合はコード数に比例する走査がありますが、他講義の計画本文を読み取り・複製しません。複数の同等コードがある場合は完全一致を優先し、それがなければ SQLite のコード順で最初を選び、以前の HashMap の列挙順による曖昧さを除去します。コード選択と行取得は同じ読み取りトランザクションで行います。設定変更・新しい索引・保存済みコードの書き換えは追加しません。

対象計画の行変換や SQL の失敗は読み取りエラーとして返し、LIVE の「読み込み失敗」という参考情報に反映します。壊れた行を黙って省略し、残った行だけで完全な計画と扱いません。独立して読み取れるシラバスの補足は保持します。全文用・時間割用の既存の授業計画 API は継続して使用できます。

回帰テストは一時 SQLite DB で全文・順序・更新・引用符を含むコード、Unicode の空白、同等コードの安定した優先順位、対象行の失敗と再試行、旧データを保持した再オープンを確認します。製品の SQL の実行計画で通常取得に索引検索を使い、一時ソートを行わず、代替コードの走査にもコードだけの covering index を使うことを確認します。別の接続から更新する読み取りスナップショットと、変更前の全取得による LIVE の参考情報・TODO の system/user メッセージ全体との一致も比較します。壊れた対象行の扱いと曖昧なコードの優先順位は上記の改善に従います。

手動測定は `cargo test --manifest-path src-tauri/Cargo.toml --lib benchmark_one_course_plan_read -- --ignored --nocapture` です。2026-10-08 の macOS、非最適化の test profile、一時 SQLite DB、各講義 18 行で、ウォームアップ後に新旧の実行順を交互にした九回の中央値を比較しました。通常コードの読み取りは 16 講義で 0.472 → 0.031 ms、256 講義で 7.273 → 0.034 ms でした。Unicode の空白付きコードの互換読み取りはそれぞれ 0.464 → 0.068 ms、7.242 → 0.532 ms です。共有 DB のロック・行変換・不要な本文の破棄を含む局所的な読み取り時間であり、fixture 作成、worker の待機、TODO プロンプト、モデル、UI、アプリ全体の RSS や GPU を測定した値ではありません。

### AI 設定と呼び出しの受付

`ai/operations.rs` は `get_ai_config`・`save_ai_config`・`ai_chat`・`ai_test_connection` を IPC の受理時に同期でキューへ登録します。命令名・`config` と `messages` の入力・設定オブジェクト・保存の null 応答・モデルの文字列応答は保持します。元の IPC メッセージを worker へ移し、設定と全文・画像の所有値への変換、設定ファイルと資格情報の読み書き、保存前の正規化・検証、設定変更イベントと LIVE の周期処理への通知を blocking worker で処理します。設定と AI 応答の JSON 化も共通の `background_ipc.rs` を使い、完了待機の async スレッドで再度シリアライズしません。

`background_queue.rs` は従来の Agent の変更キューから共用化した FIFO 実装です。Agent の DB 変更と AI 設定の準備には別々のインスタンスを使います。設定操作は受理した順に処理し、完了 Future の初回 poll・逆順の待機・待機元の破棄で順序を変えません。保存は従来の provider・URL・モデル名の規則で検証し、成功した場合だけイベントを発行します。個別のエラーや panic はその応答に返し、後続の準備を止めません。

モデルに渡す設定は準備 worker が一度読み取った所有値です。モデルの通信はキューから出た後に async で待つため、推論中も後続の設定保存・読み取りを実行できます。設定の準備だけを退出の待機対象にし、モデルの通信完了は退出条件へ追加しません。内部の他の設定読み取りまでこの FIFO に入れる変更ではなく、資格情報と設定ファイル全体の原子的な保存も保証しません。

回帰テストは単一 async スレッドと実際の一時 JSON ファイルで、遅い準備中の別タスク、未 poll・破棄された保存応答、後続読み取りが見る最終設定と通知、保存エラー・panic 後の継続を確認します。設定・全文・約 2 MiB の画像の入力、設定オブジェクト・null・長い Unicode 応答とエスケープを比較します。代替モデルを待機させている間にも設定を更新でき、既に準備したモデルの設定と言語が変わらず、設定読み取りが一回であることを確認します。ユーザーのキーチェーン・外部 AI・実機の UI や GPU を使う試験ではありません。

### 非ストリーミング AI の HTTP 準備と解析

LIVE・時間割・通常の AI が使う `ai/completion.rs` のクラウド呼び出しでは、`completion/requests.rs` の blocking worker が URL・ヘッダー・provider ごとの本文を組み立て、Reqwest の JSON エンコードまで行います。取得済みの小さい設定を複製し、全文・画像のメッセージ配列は所有権を移して渡します。Gemini の既存の文面変換と本文の項目、OpenAI 互換の既存の入力形式、認証ヘッダーと `Accept-Encoding: identity` は保持します。共有 HTTP client の接続プールは引き続き使い、ネットワークの待機は async に残します。

完全な応答の JSON 解析、内容の取り出し、最終 API エラーと再試行時のエラー本文の要約は `completion/processing.rs` の worker で処理します。OpenAI の文字列・配列・reasoning の互換処理、Gemini の最初の candidate と最初の text part、空応答と解析エラーの扱いを保持します。本文や入力を追加で切り詰めず、CPU 処理とその一時データの破棄を async 実行スレッドから移します。Gemini の status エラーの文字数制限は Unicode 境界で行い、バイト途中の切り出しによる panic を除去します。

通信・本文読み取りの失敗、429・5xx の四回までの再試行と既存の backoff・Retry-After の規則は保持します。再試行ではエンコード済みの Reqwest body を共有して複製し、全文を再エンコードしません。待機元を破棄しても開始済みの blocking 処理は実際の終了まで進むため、準備の worker を瞬時に停止できるとは扱いません。準備の async 継続を破棄した場合は、結果から後続 HTTP の段階へ進みません。

回帰テストは変更前の組み立てと解析をテスト内に残し、全文・数 MiB の画像、roles・system 指示、URL のエスケープ、ヘッダーと JSON 本文の全バイト、応答の配列・reasoning・エラーの一致を比較します。単一 async スレッドと遅い準備 worker で別タスクの実行、メッセージと画像の元のバッファの所有権移動、worker の失敗・待機破棄を確認します。localhost の HTTP サーバーには実際の製品の呼び出しから 429・503・成功を順に返し、三回の要求の全文とヘッダー、最終応答が一致することを検証します。外部 AI サービス・ユーザーの資格情報・実機の UI や GPU は使用せず、アプリ全体の CPU/RSS の改善率を測定した試験ではありません。

### ネイティブ字幕のイベントと待機キュー

macOS と Windows の字幕浮窗は `subtitle_events.rs` の共通ゲートを使います。`live-transcript-appended` の確定行と `stt-partial` の仮字幕は現在の LIVE 録音 ID と照合し、確定行は STT の取得順序番号も受け取ります。重複していた `live-line-appended` の発行と浮窗の `stt-final` 購読を除去し、表示する確定字幕はバックエンドへの追加が済んだ行に限定します。保存される履歴はすべて保持し、表示の統合とは分離します。

短い判定の間だけ LIVE ロックを保持し、その中で共通ゲートを更新します。以前の録音 ID の読み取りが遅れてゲートを過去の所有者に戻すことを防ぎ、UI 操作とイベント通知はロックを離してから行います。録音の切り替えは順序・仮字幕の頻度制御をリセットし、すでに待機中の旧字幕も世代で無効化します。仮字幕の間隔は `Instant` の 120 ms で測り、壁時計の変更に左右されません。頻度制御で表示しなかった新しい仮字幕も、古い確定行の上書きを拒否するための順序には反映します。

文字更新は一つのメールボックスに最新の字幕だけを保持し、主スレッドへの未処理の文字更新要求を統合します。待機するクロージャーは全文ではなく小さなチケットだけを持ちます。閉じる操作・送信失敗・遅れて実行される旧チケットが、再度開いた浮窗の新しい字幕を消費しません。Windows のウィンドウ作成中も同じチケットで待ち、最初の字幕を捨てません。所有者が変わったときは旧表示を消し、自動非表示も実行時に現在の所有者を確認します。

回帰テストは 1,000 件の待機更新の統合、8 スレッドからの 1,600 件の順序競合、旧所有者・重複・遅延・閉じる操作・送信失敗、実際のデルタ JSON の所有者と順序を検証します。これは表示キューの検証であり、実機の WebKit GPU 障害が解消した証明ではありません。

### Agent のクラウド HTTP 準備と応答処理

`agent_provider/requests.rs` は OpenAI 互換・Gemini の計画とストリーミング要求の URL、認証ヘッダー、system 指示、全文・画像の JSON 本文を blocking worker で組み立て、Reqwest のエンコードまで行います。取得済みの小さい設定だけを複製し、元のメッセージ配列は `Arc<Vec<ChatMessage>>` に所有権を移します。ストリーミング、画像非対応の再試行、空の表示回答に対する非ストリーミングフォールバックは同じ読み取り専用の全文・画像を共有し、フォールバックのために履歴と Base64 を深く複製しません。エンコードに必要な一時 JSON 値の作成と破棄は worker 内です。

`agent_provider/processing.rs` は非ストリーミングの全文応答を所有値として受け取り、JSON 解析、文字列抽出と一時 JSON 値の破棄を worker で処理します。OpenAI の content 配列と reasoning 回収、Gemini の全 text parts と functionCall・MALFORMED_FUNCTION_CALL、従来の空応答と解析エラーを保持します。SSE の読み取りと小さい受信イベントの処理、HTTP の待機は async に残します。

計画の既定上限 8192、回答の既定上限 32768、JSON 出力指定、画像非対応と response_format 非対応への切り替え、計画だけの `Accept-Encoding: identity` は保持します。Agent 固有の HTTP client と既存の再試行規則も維持します。Agent の非ストリーミング要求は送信の通信失敗でのみ合計二回まで試行し、本文の途中切断・429・5xx を通常 AI の四回再試行へ統合しません。空のストリーミング回答のフォールバックは元の画像を含む入力から始め、擬似ツール呼び出しは本文の callback に表示しません。

回帰テストは変更前の四つの request builder と二つの response parser を残し、空入力、複数の system 指示、Unicode と大きい全文・画像、roles、URL、認証ヘッダー、JSON 本文の全バイトと応答を比較します。元の配列・本文・Base64 のバッファ共有、遅い準備中の別 async タスクの進行、worker panic と要求の置換による取消を確認します。取消で準備の async 待機を破棄すると、開始済みの worker は終了しても後続の送信へ進みません。localhost の実際の provider 呼び出しで画像と JSON 形式への切り替え、空 SSE のフォールバック、reasoning と表示 callback、擬似ツールの非表示を検証します。外部 AI・ユーザーの資格情報・実機の UI/GPU は使わず、アプリ全体の RSS や白画面障害の解消を測定した試験ではありません。

### Agent の SSE データ受信

OpenAI 互換と Gemini の回答は `agent_provider/sse.rs` の共通受信器を使います。旧実装は HTTP チャンクごとに UTF-8 を lossy decode していたため、漢字・絵文字の途中で分割されると置換文字が混入しました。共通受信器は未完の行を元のバイトで保持し、行が完成した後に UTF-8 を解釈します。完成した行は受信バイトから借用し、一行ずつ残り全体を新しい文字列へ複製しません。改行の検索で過去の未完行を再走査せず、未完行と現在の data のバッファを再利用します。

[SSE の data framing 規則](https://html.spec.whatwg.org/multipage/server-sent-events.html#event-stream-interpretation)に従い、LF・CRLF・単独 CR、先頭の BOM、コメント、コロン直後の省略可能な一個の空白、多行 data と空行によるイベント確定を扱います。AI 呼び出しが使わない event・id・retry の metadata は無視し、EventSource の自動再接続は追加しません。EOF 前に空行で確定しなかったイベントは表示へ渡しません。OpenAI の `[DONE]` は行処理だけでなく HTTP 受信全体を終え、同じチャンク内や後続の内容を表示しません。Gemini の functionCall と MALFORMED_FUNCTION_CALL も受信を終えて executor 用の呼び出しを返します。取消は各チャンクと各イベントの前に確認し、最初の callback による取消後は同じチャンクに残る次のイベントを拒否します。既存の要求所有者による通知取消、thinking の分割、本文 callback、画像降級と空回答のフォールバックは保持します。

回帰テストは旧実装で一バイトチャンクが文字を壊すことを再現し、全ての切断位置、1～31 バイトの固定チャンクと可変チャンクで Unicode・BOM・CRLF・多行・不正 UTF-8 の結果を検証します。通常の provider データは旧実装と全文を比較し、4000 イベントをまとめた入力で完成行を保持せず、data のバッファを再利用することも確認します。localhost の chunked HTTP 応答を実際の provider 受信関数へ渡し、OpenAI の content と reasoning、Gemini の全 text parts と二種類のツール呼び出し、DONE の後のデータ拒否、HTTP の終了を待たない接続解放、未確定イベントの破棄、通信エラーと callback 中の取消を検証します。

手動測定は `cargo test --manifest-path src-tauri/Cargo.toml --lib benchmark_sse_coalesced_batch -- --ignored --nocapture` です。2026-10-08 の macOS、非最適化の test profile、ウォームアップ後に新旧の順序を交互にした九回の中央値で、一つのチャンクにまとまった 2000 イベント・114000 バイトは 5.732 → 0.742 ms、8000 イベント・456000 バイトは 50.376 → 2.986 ms でした。イベントの分割と data callback だけの局所測定であり、JSON 解析、取消確認、HTTP、モデル、UI、アプリ全体の CPU/RSS や GPU 障害を測定した値ではありません。

### Agent の thinking 分割

クラウド回答と空回答フォールバックが使う `ThinkFilter` は `agent_provider/stream/think_filter.rs` に分離し、HTTP の dispatch から独立して検証できます。通常の表示本文と inline の thinking 本文を内部 String のスライスから同期 callback へ渡し、送信する断片ごとの `to_string` を除去します。callback が戻ってからバッファを drain するため、参照は呼び出し中だけ有効で、次の断片による変更を callback の後に行います。末尾の flush は従来の `mem::take` を保持し、callback の panic 前後のバッファ変更順序も変えません。

三種類のタグと既存の不完全な開始 prefix、10 bytes を基準に UTF-8 境界で保留する規則、各 callback の本文・thinking flag・順序は保持します。provider が thinking として渡した断片はタグ解釈せず直接 callback へ渡し、保留した通常本文の状態を変えません。共有 Mutex と feed/flush の二つのハンドル、空の upstream thinking、繰り返す flush、flush 後の feed、既存の poisoned lock の扱いも維持します。通信形式、取消、画像再試行、表示通知の頻度と全文は変更しません。

凍結した `stream/think_filter/before.rs` は製品には含めません。差分テストはタグ・Unicode・不完全なタグ・複数のブロックと既存の入れ子の扱いを、全ての二箇所の UTF-8 分割位置と一文字ずつの入力で比較し、連結後の本文だけでなく callback 列全体を確認します。upstream thinking と flush の混在、表示前・thinking 内・タグ境界・末尾 flush の callback panic とその後の入力も比較します。通常の drain が複製した文字列ではなく、変更前の内部バッファを callback へ借用することも検証します。

`cargo run --manifest-path src-tauri/Cargo.toml --example benchmark-think-filter` は製品のフィルターと旧実装を使う独立した比較です。通常の最適化なし dev profile、ウォームアップ後に新旧の実行順を交互にした九回の中央値で、入力の準備を除き、Arc/Mutex と二つの closure の構築・フィルター・順序と flag を含めた SHA-256 の受け手を測ります。分配は別の呼び出しで計測し、出力のサイズ・callback 数・SHA-256 が一致し、計測後の所有分配が残らないことを確認します。SSE の framing・JSON・ネットワーク・モデル・アプリ・実機 UI/RSS/GPU、入力と allocator 管理領域・realloc 内部は計測しません。

2026-10-08 の macOS ローカル測定で、一万片の Unicode 表示本文は 10,001 callback・368,890 bytes が一致し、分配要求が 10,008 → 8 回、総要求量が 369,356 → 478 bytes、中央値が 21.979 → 21.323 ms でした。inline と upstream thinking の混在は 13,750 callback・165,000 bytes が一致し、12,508 → 9 回、124,233 → 493 bytes、18.238 → 17.417 ms でした。320,000 bytes を一片で渡す例はピーク要求量が 640,364 → 320,376 bytes、中央値が 11.753 → 11.760 ms でした。小さい断片では同時に保持する一時文字列も小さいため、分配回数の減少をアプリ全体の常駐メモリの減少へ換算しません。

### Agent の疑似ツール呼び出しの検出

表示前の疑似呼び出し検出は `agent_pseudo_call/detection.rs` に分離し、引数の解析・ツール名の解決から独立して検証します。以前は候補境界ごとに残りの全文を小文字の String に複製し、空白と包装符の連続を各候補から再び走査していました。現在は UTF-8 の文字を一度だけ前方へ走査し、包装符と空白の連続に入る最初の有効なバイト位置を保持します。共有の九種類のマーカーは固定長のバイト prefix を ASCII 大文字小文字を区別せず比較し、残りの本文を複製しません。

Unicode の空白、バッククォート・`<`・`‹`・`〈` の包装、括弧・引用符の後の境界、最初に返すバイト位置は維持します。検出は大文字小文字を区別しませんが、実際の呼び出し解析のマーカーは従来どおり区別します。全角のコロン、未知のツールの抑制、引数・ツールの検証も変えません。ストリームの保留文字数、表示 callback の順序とバッファの変更順序はこの変更の対象外です。

`agent_text::visible_without_thinking` は開始タグのない本文を `Cow::Borrowed` として参照し、思考タグがある場合だけ従来と同じ可視本文を所有 String に生成します。検出と疑似呼び出しの解析はこの関数を使い、タグのない全文の一時複製を除去します。既存の `strip_think` は所有 String を返す API のままです。三種類のタグと不完全な開始 prefix、未終了・入れ子の処理は保持します。

凍結した `detection/before.rs` はテストと独立した比較プログラムだけで使います。差分テストは全マーカーの大小文字、Unicode の空白と包装、各候補境界、思考タグと各 UTF-8 切断位置、4,000 件の再現可能な混在入力について、真偽値だけでなく返すバイト位置と可視本文を比較します。長い連続包装符と通常本文、タグなし入力の参照、全角コロン・Unicode 引数の解析と既存の大小文字の扱いも確認します。

`cargo run --manifest-path src-tauri/Cargo.toml --example benchmark-pseudo-detection` は製品の検出関数と凍結した旧実装を比較します。最適化なし dev profile、三回のウォームアップ後に新旧を交互にした九回の中央値を取り、分配は別に計測します。通常の文章、末尾の疑似呼び出し、連続包装符、思考タグあり・空入力の結果を照合します。`find_start` と先頭の検出は所有分配ゼロ、`has_any` はタグなしでゼロ、タグありで可視本文の一回だけの分配を確認します。入力の準備・SSE・JSON・ツール引数の解析・IO・ネットワーク・モデル・アプリ・UI/RSS/GPU と allocator 管理領域・realloc 内部は測りません。

同じ比較プログラムは標準ライブラリーだけで構成し、`rustc --edition=2021 -O src-tauri/examples/benchmark-pseudo-detection.rs -o /tmp/selah-pseudo-detection-optimized` と生成した実行ファイルで最適化後も比較できます。2026-10-08 の macOS の最適化後の局所測定では、77,824 bytes の普通の文章を `has_any` で検査する中央値が 7.130 → 0.111 ms、分配要求が 6,145 → 0 回、総要求量が 239,163,392 → 0 bytes でした。この総量は短命な分配を足した値で、同時に保持するメモリではありません。旧実装のピーク要求量は 155,648 bytes でした。末尾に呼び出しがある 77,875 bytes は 7.183 → 0.123 ms、6,146 → 0 回でした。包装符が連続する 6,152 bytes は 7.151 → 0.005 ms、3,074 → 0 回、思考タグを含む 48,143 bytes は 0.459 → 0.043 ms、1,537 → 1 回でした。これは当該テキスト関数の比較であり、回答待ち時間全体や白画面の改善を保証する値ではありません。

### Agent の保存と起動準備

ネイティブショートカットの終了コールバックは、発話・会話 ID・要求 ID を共通状態で予約し、表示要求と `native_agent_submission.rs` の背景タスクを登録して戻ります。会話と全文の最初のユーザーメッセージを一つの SQLite トランザクションで保存します。メッセージ保存に失敗すると会話の作成もロールバックし、空の会話を残しません。失敗した発話は `native_agent_submission/storage.rs` に保持し、次の退出要求で再試行します。実行中の保存は再試行せず、保存完了・unwind 後の再試行は同じ会話 ID と最初のユーザーメッセージを照合します。保存後の手動タイトルや後続の会話を上書きしません。保存は blocking pool で行うため、SQLite のロックや書き込みが STT の `idle` コールバックを占有し、停止完了の通知を遅らせません。保存失敗や worker の終了はその会話の Notice に反映し、保存に失敗したまま推論を開始しません。表示を閉じても受け付けた発話の保存と提出は続け、遅い回答・エラーは会話 ID と要求 ID が一致する表示だけに適用します。回答は二つの OS で共通の型付き sink に直接渡し、動的な回答 JSON 購読を登録しません。推論より先に保存を完了し、保存した入力だけが所有権付きの `SavedVoiceInput` を生成します。Agent はこの複製できない入力を一度消費し、同じユーザーメッセージを再度保存しません。別の会話への流用はエラーにします。

Agent 全体の初期準備も `agent/prepare.rs` の blocking pool を使います。ユーザーのメッセージと画像を保存し、タイトルを更新してからプロバイダーを解決し、履歴を読みます。キーチェーン・設定・SQLite の処理を async 実行スレッドから移し、AI が無効・モデルの起動不可でも受け付けたメッセージを履歴に残します。履歴の読み取り失敗は空の履歴で処理を続けず、エラーを返します。自動タイトルは会話一覧の全取得を除去し、既定のタイトルだけを変える一回の条件付き UPDATE を使います。途中の手動変更を自動タイトルで上書きしません。保存後には会話一覧の変更を一度通知するため、固定の音声タイトルやプロバイダー起動失敗の場合も保存済みの会話が一覧に反映されます。

最初の履歴読み取りは全文ではなく、保存トランザクションが返した入力のメッセージ ID より小さい行だけを SQLite の索引から取得します。履歴の最終行が現在の入力であるとは仮定しません。通常入力と保存済み音声の両方が実際の保存 ID を持ち、音声の再試行も最初の保存 ID を返します。入力 ID は要求の所有者に一度だけ設定し、別会話・ユーザー以外・未保存・削除済みの入力はエラーにします。取得数はモデルの履歴窓とクリック確認・番号選択に必要な過去行数の大きい方で、現在は入力を除く最新 11 行です。同時刻の行は ID で並べ、直前 10 行を借用するため、画像・ツール JSON を含む履歴の再複製も行いません。現在の本文と添付は履歴とは別に一度だけモデルへ渡します。会話画面の全履歴と保存内容は保持します。後続の再計画は `agent_load_turn_planning_messages` で最近 8 行と必要ならそれより前の最新画像を取得します。対象は入力より前に保存された行と、その要求が保存に成功したツール・回答の ID だけです。別の要求が後から保存した入力・回答・スクリーンショットを、テキスト窓にも旧画像の検索にも混ぜません。ツールを省略する判定に必要な 6 行も確保します。画像が最近の行にある場合と非画像モデルの場合は過去を検索しません。古い画像は `(created_at, id)` の索引で候補を新しい順に読み、最初の有効な画像で止めます。ツール名に限定しないため、別のツールが返した画像も従来どおり参照します。選んだ旧画像行はテキストの窓より前に置き、画像の追加で古い文章をプロンプトに混ぜません。この読み取りも blocking pool で行い、独立した読み取り専用接続のトランザクションを使います。最近の行と旧画像は同じ SQLite スナップショットから選ぶため、途中の書き込み・削除で混在しません。画像の解析・検索中は共有 DB 接続の Mutex を保持しません。選択した候補 JSON の所有文字列は再読・複製せずに結果へ渡します。読み取り・行変換に失敗した場合、空の履歴や欠落したメッセージで継続しません。

一時 DB に 2,000 行・各 16 KiB のツール履歴を作る手動ベンチマークは `cargo test --manifest-path src-tauri/Cargo.toml --lib benchmark_recent_agent_history_reads -- --ignored --nocapture` で実行します。さらに入力後の別要求の 32 行を保存し、製品の全文取得後の ID 境界による選別と、入力を検証して過去 11 行だけ取得する SQL を交互に測定します。初回を除いた各 7 回の中央値を出します。これは DB の読み取り・行生成の比較であり、AI 応答、アプリ全体の CPU/RSS、GPU 障害の改善率ではありません。

後続の読み取りの手動ベンチマークは `cargo test --manifest-path src-tauri/Cargo.toml --lib benchmark_planning_history_reads -- --ignored --nocapture` です。過去 2,000 行・各 16 KiB に本要求の回答 2 行と別要求のスクリーンショット 32 行を追加し、テキスト窓の近く・最初の行・画像なし・非画像モデルの四条件を使います。独立した接続を開く費用、製品の画像検索・プロンプト生成を含めて全文取得後に同じ入力境界と所有 ID で選別した計画コンテキストと比較します。回帰テストは全文から作ったローカル・画像モデルのメッセージとの一致、非画像モデル、最近・旧・不正な画像、会話の分離、同一時刻の順序、検索中の会話削除と共有 Mutex の解放を確認します。旧画像が非常に古い場合や画像がない場合は途中のツール JSON を順に確認します。画像のない大きな結果を毎回 JSON 解析しないよう、画像キーの候補を先に確認します。未エスケープの `"image"` もバックスラッシュもない JSON だけを除外し、エスケープされたキーを含む可能性がある結果は完全なデコーダーへ渡します。候補の有無だけで画像を受け入れず、従来どおり全 JSON・フィールド型・末尾を検証します。回帰テストは Unicode エスケープのキー、重複キー、入れ子の画像、本文中の文字列、不正な末尾、大きな本文について全文デコーダーと比較します。保存された全文を削ったり、一定件数で画像の検索を打ち切る処理は行いません。

ツール結果の JSON 変換と SQLite 保存、通常・ブラウザー直接回答の保存は `agent/persistence.rs` で共有し、blocking pool で実行します。大きい結果は所有権を worker に渡し、保存後に同じ値を返すため、スクリーンショットの文字列を丸ごと複製しません。通常のツール、失敗後に省略した操作、自動ブラウザー操作、自動ノート読み取りのいずれも保存失敗を返し、次の計画へ進みません。成功したコミットの ID だけをその要求の所有者に記録し、読み取り時は小さい ID 一覧だけを複製します。背景の保存 worker は元の所有者を保持するため、新しい要求が始まった後に完了しても新しい要求の履歴へ登録しません。所有者のない再計画・結果保存はエラーにし、全会話の履歴へ暗黙に戻しません。要求情報はメモリ内に保持し、既存の DB スキーマは変更しません。これは既に実行済みのツール操作を取り消す処理ではありません。

回帰テストは単一の async 実行スレッドと遅らせた保存処理で、待機中も別のタスクが実行できること、保存完了前や失敗後に推論を始めないこと、保存待機中の表示の入れ替えと遅い回答・エラーの隔離を確認します。実際の一時 SQLite DB と製品のメッセージ保存処理で、プロバイダー解決の失敗後も全文と画像 JSON が一度だけ残ること、既定タイトルの更新と手動タイトルの保持を確認します。追加の回帰テストは SQLite のトリガーでメッセージ保存を故意に失敗させ、会話のロールバックと同じ ID による再試行、保存済み ID の重複拒否、推論待機前の全文保存、プロバイダー失敗後の一件だけの履歴、別会話への入力の流用拒否を確認します。実際のマイク、ユーザーのキーチェーン、AI サービス、GPU は起動しません。強制終了・電源断を、この保存順序のテストで保証したとは扱いません。

履歴の回帰テストは同じ本文の並行入力、同時刻・システム時刻の巻き戻り、音声の再試行後の後続入力、古い要求の遅いツール・回答、旧画像の選択、保存失敗時の ID 非登録を確認します。全文から対象行だけを選んだ参照履歴と、製品の SQL が生成するローカル・画像モデルのメッセージと省略判定を比較します。対象外の不正な列は変換せず、対象列の変換失敗はエラーにすること、索引を使い追加ソートをしないこと、入力検証と旧画像の選択が同じ読み取りスナップショットを使うことも検証します。境界は入力のコミット順序です。IPC の受理順序と背景 worker の保存順序を同じにする保証ではありません。

### Agent の要求ごとの推論と取消

`agent_turn_scope.rs` は一回の要求に一つの所有者を割り当てます。同じ会話の新しい要求を受け付けると、それまでの要求を取り消し、新しい要求を現在の所有者にします。別の会話の推論は独立しています。会話 ID は履歴とイベントの宛先に使い、モデルの推論・計画の取消には要求ごとに新しく生成する内部 UUID を使います。モデルの呼び出し開始・終了による取消フラグの解除が、古い要求の取消を復活させません。

`agent_commands/submission.rs` の IPC adapter は通常・ページ文脈付き送信を受けた時点で、引数と対象ページの検証、入力保存枠の予約、要求の登録、blocking 準備 worker の提出を同期的に済ませます。`Admission` を使い、完了応答の Future が初めて実行される前から取消対象と保存 worker が存在します。Tauri の既定の async 命令だけでは、その命令の Future が実行される前に同期の取消命令が届く可能性があるためです。古い完了 Future が遅れて始まっても、要求を再登録して現在の要求を奪いません。二つの IPC 名と省略可能な引数、推論完了まで待つ Promise の返し方は保持し、その他の命令は従来の generated handler に渡します。Tauri の public な応答 resolver を使い、入力のコピーや SQLite、キーチェーンの処理を IPC の同期区間で実行しません。

同期の引数検証は元の JSON の文字列を借用します。本文と全画像の MIME・base64 は blocking worker で所有文字列に変換し、入力の保存・プロバイダーの解決・初期履歴の取得を同じ worker で続けます。元の IPC メッセージはこの復号処理で消費し、モデルの処理へは保存した入力と必要な履歴だけを渡します。空の入力、不正な添付・文脈、存在しない対象ページ、閉じた保存ゲートは要求の登録より先に拒否し、実行中の有効な要求を置き換えません。完了応答を実行前に破棄しても、提出済み worker が保存枠と所有者を持って入力を保存し、その後に取消を確認して推論を省略します。強制終了や電源断時の復元を保証する処理ではありません。

主画面と側欄は送信時に要求 UUID を生成し、通常・ページ文脈付き送信に `turnId` を渡します。バックエンドは既存の各ストリームイベントのフィールドを保ち、同じ UUID を `turn_id` として追加します。画面は現在の UUID と一致するイベントだけを処理するため、同じ会話の旧 token・plan・エラー・終了や、別の画面からの要求を混ぜません。停止・選択変更・画面の破棄でも、その画面の要求 UUID を取消命令へ渡します。新しい要求が始まった後の旧取消は、その新しい要求を取り消しません。省略した旧命令では会話の現在の要求を取り消し、送信側が ID を省略した場合はバックエンドが生成します。ネイティブ音声提出は従来の固有の会話 ID を使い、追加フィールドを読み飛ばす回答解析と互換です。

パイプラインの async タスクには `tokio::task_local!` で所有者を保持します。この値は新しいタスクや blocking worker へ自動継承されないため、保存前準備・ローカルモデル worker・回答 callback は明示的な `Arc` を持ちます。登録表は弱参照だけを持ち、最後の worker・callback の終了時に自分の世代の取消状態を除去します。旧 worker の終了で新しい要求の登録を消しません。要求の async タスクを破棄しても、残った worker に対する取消を保持します。通常終了・エラー・タイムアウトの終端イベント後にも要求を閉じ、時間切れ後の callback を画面へ流し続けません。Apple の `TaskRegistry.insert` も、Task の生成後・登録前に届いた取消を登録時に確認して、その Task に反映します。

取消の通知は待機元を起こし、HTTP ヘッダーや次の SSE データが届くのを待たずにプロバイダーの async 待機を終了します。通常・自動実行ツールも同じ通知と既存の時間制限を使い、待機中の 120 ms ごとの取消 polling を除去しました。自動ノート読み取りは共通のツール実行・保存処理を使います。待機を中断しても、既に開始した blocking 処理・外部サービスの操作を瞬時に停止したり取り消したりする保証ではありません。ユーザー入力と添付は従来どおりプロバイダーの取消確認より先に保存し、既に実行中の保存を取り消しません。

受付の回帰テストは Tokio のタスク外から製品の `Admission` を使い、完了 Future を一度も実行する前の取消、準備の完了順を逆にした同じ会話の二要求、旧取消の拒否を確認します。完了 Future を初回の実行前に破棄した場合も、製品の `TurnInput`・本文保存 API と一時 SQLite DB で、全文・画像の一回だけの保存、退出の保存枠の完了、所有者の後始末を確認します。2 MiB の添付を含む実際の IPC body では、同期検証が元の文字列を借り、復号後も全画像・全文・ページ文脈を保持することを比較します。省略・null・画像だけの入力、余分なフィールド、通常送信がページ引数を無視する既存の扱い、不正な文脈と保存ゲートの拒否も検証します。OS の実際の WebView IPC を遅らせる統合試験や、アプリ全体の CPU/RSS の測定ではありません。

回帰テストは同じ会話の八スレッドからの競合、会話間の独立性、過期の取消、計画・回答双方のフラグ解除、複数の待機元、async タスク破棄後も残る blocking worker と最後の参照の後始末を確認します。実際のプロバイダー経路を localhost の HTTP サーバーに接続し、応答ヘッダー前と SSE の途中で停止して、要求の終了と接続の閉鎖を確認します。ツール待機の取消・後続実行の拒否・既存のタイムアウト結果、一時 SQLite DB への全文・添付の保存も検証します。主画面と側欄の実際の TypeScript 関数を代替 IPC で実行し、遅いイベントと重複した終端が新しい要求を変更しないことを確認します。`scripts/test-apple-task-registry.sh` は製品の Swift 登録クラスを抽出してコンパイルし、登録前・登録後の取消と再利用、200 回の並行登録・取消を確認します。Svelte の実際の描画、Apple のモデル、マイク、外部 AI サービス、アプリ全体の CPU/RSS や WebKit GPU 障害の解消を検証したものではありません。

### Agent の添付ファイル

主チャットとサイドパネルは `agentAttachments.ts` の読み取りと追加処理を共用します。画像に加え、PDF、DOCX、PPTX、XLSX、TXT、Markdown、CSV、TSV、JSON、LOG、YAML に対応します。添付は合計四件・一件 10 MiB です。画像は元の Base64 と MIME、文書はファイル名・MIME・元のサイズ・抽出本文・部分読み取りのフラグを型付きで送ります。MIME が空または `application/octet-stream` の画像は既知の拡張子から補います。非対応形式・空ファイル・サイズ超過・読み取り失敗や中断・不正な data URL は入力部にエラーを表示します。

`agent_read_document_attachment` は Base64 の長さを割り当て前に確認し、デコードと文書解析を blocking worker へ移します。PDF のテキスト層は最大百ページ、本文は一件六万 Unicode scalar まで取り込み、切り詰めや失敗したページがあれば部分読み取りと表示します。スキャン PDF の OCR、旧式 DOC/XLS、数式再計算は行いません。Office は ZIP 内の本文 XML を読み、展開サイズを一部品二 MiB・合計八 MiB・四千 entry に制限します。段落、改行、タブ、XML 文字参照を保持し、PPTX はスライド番号順、XLSX は共有文字列を実際の文字へ戻し、セル番地と値・未計算の数式を並べます。Excel の表示書式や日付書式の再現ではありません。テキストは UTF-8 または BOM 付き UTF-16 を受け付け、文字コードを推測して内容を置換しません。プレビューは最初の二千文字を表示します。

入力部は `AgentAttachmentStatus.svelte` を共有し、対応形式と四件・10 MiB の制限を常時表示します。読み取り中は送信できないこと、完了後は準備できた添付の件数と送信時に Agent へ渡すことを表示します。文書カードにはファイル名と本文の読み取り完了または部分読み取りの状態があり、プレビュー二千文字の省略と解析自体の切り詰めを区別します。エラーは失敗したファイル名を含み、混在した選択で成功したファイルを失いません。読み取り中の表示と失敗を同時に示し、削除操作では古い上限エラーを消します。履歴の文書説明は気泡の文字色を継承し、ライト・ダークテーマの気泡でも状態を読めます。

読み取りと文書解析が終わるまで新規送信を受け付けず、操作ボタンにもその状態を反映します。追加の直前に四件の上限を再確認し、並行した選択・貼り付けで超過しません。ページ破棄後に完了した読み取りは添付やエラーを更新しません。ファイル選択値は待機前にリセットし、同じファイルを再選択できます。会話準備中に追加した添付と編集した本文は次の入力として保持し、送信時に取得した添付だけを消費します。

文書だけの要求も受け付け、保存予約と要求 ID の既存の受付経路を使います。モデル用の本文には文書名と抽出本文を付加し、文書付きの Apple 用回答には従来の四百文字より広い千八百文字の入力枠を使います。モデル自体のコンテキスト制限は残ります。`agent_messages.documents_json` に元の入力本文と型付き文書を保存し、モデル用本文・画像・文書メタデータは一つの INSERT と会話更新のトランザクションで確定します。既存 DB は列と部分索引を追加するだけで移行します。表示履歴は専用の文書メタデータを取得し、元のユーザー本文と添付プレビューを復元します。旧メッセージの JSON には空の文書フィールドを追加しません。原本の PDF/Office のバイナリを保管する仕組みではありません。

`tests/agent-attachments.test.mjs` は両コンポーネントの TypeScript と共有処理を実行し、MIME 補完、失敗・中断と再試行、四件上限、破棄後の完了、未完了の添付を送らないこと、文書だけの送信、会話準備中の追加を検証します。`scripts/check-agent-attachments-browser.mjs` は実際の Svelte コンポーネントを描画し、実際の FileReader と合成ファイル、JSON の代替 IPC で入力・プレビュー・状態表示・削除・再試行・送信・履歴再読を確認します。幅の狭いサイドパネルの折り返し、Unicode のプレビュー上限、文書内 HTML の文字表示、ライト・ダークテーマの気泡の説明色も検証します。Rust では実際の PDF と ZIP の小さな合成資料、Unicode と UTF-16、XML 文字参照、セル参照、サイズ上限、入力受付、SQLite の再起動後の復元と書き込み失敗、旧 DB 移行、MockRuntime のコマンド引数を確認します。ユーザーのファイル・マイク・クラウド API は使わず、macOS のファイル選択パネルやクラウドモデルに実画像を送る検証ではありません。

### Agent の表示履歴

主画面・側欄の履歴と完了後の回読は `agent_load_display_messages` を使います。画面で表示する全ての `user`・`assistant` 行を SQL で選び、本文・添付・ID・会話 ID・時刻を同じ `(created_at, id)` の順序で返します。件数の上限や本文・添付の切り詰めは行いません。画面で使わない `tool_name`・`tool_result_json` は SQL の NULL 投影を使い、所有文字列や JSON の木を作りません。不可視のツール行も読み取り結果に含めないため、過去の画面写真や資料本文を WebView に送ってから捨てる処理を除去します。SQLite の読み取り・添付の変換・JSON エンコードは async 命令の blocking pool で行います。JSON の IPC 応答を `tauri::ipc::Response` で渡し、async 実行スレッドで再度エンコードしません。応答は従来と同じ JSON 配列であり、文字列やバイナリー配列への変更ではありません。元の `agent_load_messages` 命令、DB の全文・モデル用の最近の履歴と保存済みメッセージは保持します。リアルタイムのツール計画・進捗は引き続きストリームイベントで表示します。ネイティブ命令とフロントエンドは同じビルドで更新します。

`idx_agent_messages_display` は表示する二つの role だけを含む部分索引です。会話と時刻の索引順を使い、多数のツール行を走査・並び替えません。可視メッセージの書き込みには追加の索引更新が必要ですが、ツール結果の追加はこの索引へ入りません。既存 DB の起動時に索引だけを追加し、メッセージを移行・削除しません。回帰テストは多数の全可視行と添付の保持、会話の分離、同じ時刻・逆行した時計の順序、索引がない DB の再起動、SQL の索引利用と一時ソートの不在を確認します。不可視行・列の不正な SQLite 型を避ける一方、表示する本文・添付列の型エラーは欠落した履歴で続行せず返します。添付 JSON の変換結果は従来の DTO と比較し、不正な JSON の従来の扱いも保持します。Tauri の実際の JSON 応答をメッセージ配列として復号し、DTO との一致を確認します。

手動測定は `cargo test --manifest-path src-tauri/Cargo.toml --lib benchmark_agent_display_history -- --ignored --nocapture` です。2026-10-08 の macOS/debug、一時 SQLite DB、ウォームアップ後に順序を交互にした 7 回の中央値で、製品のクエリ・DTO 変換・JSON エンコードを含めます。2,000 行中 1,600 行のツール結果と 2 MiB の画面写真を含む条件では、全文経路の 565.01 ms に対して表示経路は 2.54 ms、JSON は 23,675,856 → 68,793 bytes でした。可視行 400 件と添付は従来の表示内容と一致します。ツール行がない 400 件では 2.49 → 2.50 ms、JSON は双方 68,395 bytes です。Tauri の worker への待ち時間、WebView の受信・描画、アプリ全体の CPU/RSS や GPU 障害の解消を測定した値ではありません。

### Agent の書き込みと削除

会話一覧・共有選択の読み取り・表示履歴・旧 API の全文履歴は `agent_commands/worker.rs` の blocking pool で処理します。SQLite の Mutex・他接続の書き込みロック待ち、添付・ツール JSON の変換を IPC/UI や async 実行スレッドで行いません。一覧・共有選択・全文履歴も背景で JSON をエンコードし、従来の配列・文字列・null として返します。一覧の行変換エラーはその会話を黙って省略せず返します。

会話の作成・選択・名前変更・削除は `agent_commands/mutations.rs` が IPC の受理時に形式を検証してから `background_queue.rs` の会話用インスタンスに同期で登録します。既存の命令名、camelCase の入力、作成時の省略・null タイトル、作成 ID と変更命令の null 応答を保持します。完了を返す async task の開始・待機順に依存せず、受け付けた順で一つずつ SQLite に反映します。処理待ちの変更がなくなるとキューの実行ジョブを終了します。履歴読み取り・モデル worker はこのキューから独立しています。ジョブの実行・通知中にキューの Mutex を保持しません。完了待機の破棄は受理済みの変更を飛ばさず、個別の DB エラーや panic でも次の変更を処理します。モデルへの送信は従来どおり別の要求所有者を使います。

通常のメッセージ追加は会話の存在確認・本文・画像・ツール結果・会話の更新時刻を一つの SQLite トランザクションで扱います。会話が存在する場合だけ `INSERT ... SELECT` で追加するため、外部キー制約が無効な接続でも、削除後の遅い入力・回答・ツール結果から孤立したメッセージを作りません。本文と更新時刻は同じ時刻を使い、更新時刻の保存に失敗すると本文もロールバックします。以前の二回の独立した書き込みと更新エラーの無視を除去しました。

同じ秒に追加する複数の結果は、変わっていない更新時刻を書き直しません。メッセージと時刻の SQL は接続の statement cache を使い、毎回の再コンパイルを減らします。キャッシュに戻すとパラメーターの binding が解除されるため、前回の大きい画像を SQL のキャッシュとして保持しません。本文の保存に失敗した場合も以前の時刻を変えません。音声の最初の会話・メッセージの保存は従来の専用トランザクションを使います。

会話の削除はメッセージ・親会話・その ID と一致する共有選択の解除を同じトランザクションで扱い、どれかに失敗すると全体を元に戻します。既に別の会話を選んでいた場合はその共有選択を変更しません。共有選択の保存も会話の存在を条件にした一回の書き込みで行い、削除済み ID の保存を拒否します。同じ選択の再保存は時刻・キャッシュ版・変更イベントを更新しません。読み取りは会話テーブルと結合し、旧アプリが残した存在しない ID を返しません。再度の削除は従来どおり成功として扱います。`agent_delete_conversation` 命令は順序付きの背景 worker で削除を完了し、その worker の中で現在の要求を無効にして推論・ツール待機を取り消します。削除に失敗した場合は要求を止めません。コミット後の無効化を async 応答の待機や WebView の取消通知に依存させず、遅い token と done/error を抑止します。ネイティブの処理中カプセルにも共通状態から直接その会話の Notice を渡し、現在の要求の所有権を解除します。別の会話・新しい表示・Listening の認識済み発話は変更せず、マイクは停止しません。その後に会話 ID 付きの `agent-conversation-deleted` と一覧変更イベントを発行します。主画面・側欄は該当する会話の表示と購読だけを除去し、一般的な空の共有選択を書き戻しません。遅い削除通知が新しい選択を解除しないためです。待機中の側欄読み取りも無効にし、再確認では空の会話を自動作成しません。入力下書き・添付・認識済みの発話を保持し、マイクを停止しません。表示していた会話の削除も、主画面で既に開始した新規作成の意図を取り消しません。大きい会話のディスク処理を UI 命令スレッドで行いません。削除済み会話への保存を拒否する処理であり、既に実行したツールの操作を取り消す処理ではありません。

一時 SQLite DB の回帰テストは更新時刻・メッセージ・削除のトリガーで故意に失敗させ、全文・画像・メタデータの保持、ロールバック後の再試行を確認します。共有選択の解除失敗でも本文・画像・親会話・キャッシュ版がまとめてロールバックすることを再接続して確認します。外部キーを無効にした二つの独立した接続でも、並行する追加・選択と削除の後に孤立したメッセージや削除済み ID の共有ポインターがないことを確認します。手動ベンチマークは `cargo test --manifest-path src-tauri/Cargo.toml --lib benchmark_agent_message_writes -- --ignored --nocapture` で、WAL/NORMAL の一時 DB に 500 件ずつ書き、従来の独立した SQL と製品の追加 API を交互に比較します。アプリ全体の CPU/RSS や電源断時の永続性の保証ではありません。

命令の回帰テストは単一の async 実行スレッドと実際の SQLite 書き込みロックで、待機中も別のタスクが動き、ロック解除後に全文と保存 ID を返すことを確認します。形式・通知・JSON 応答の互換性、逆順の完了待機・応答の破棄・エラー・panic 後も変更順が保たれること、削除待機を破棄してもコミットと要求の無効化が完了することを検証します。メッセージ・親会話・共有選択の失敗と遅延外部キーによる COMMIT 失敗では推論が維持されます。ローカル HTTP サーバーで、応答ヘッダー待ちと回答の最初の token 後に停止した実際のプロバイダーを削除で終了し、追加のサーバー応答やフロントエンドの取消 RPC なしで接続が閉じることも確認します。ネイティブ共通状態のテストは削除された処理中の要求の所有権だけを解除し、新しい表示と Listening の全文・末尾を保持することを確認します。実際の UI イベント配信・マイク・GPU 障害やプロセスの強制終了を検証するテストではありません。

### Agent のツール JSON と画像

`agent/tool_result.rs` はツール結果を借用して JSON を生成します。表示プレビューは 180 バイト、通常モデルへの結果は既存の文字数上限だけを生成し、残りの直列化を停止します。JSON エスケープのために全文を走査しないよう、文字列も必要な UTF-8 範囲だけを渡します。擬似呼び出しの変換は最長マーカー分を先読みしてから行い、境界をまたぐマーカーも従来どおり扱います。現在の置換は文字列を短くしないため、後続の大きい文章を変換用に走査・複製する必要もありません。短くする置換を追加した場合は自動的に全文を変換する経路へ戻ります。全文の `Value` を先に複製してから全文の JSON を切り詰める処理を除去しました。除外フィールドと擬似ツール呼び出し文字列の処理は従来と共通です。文字数制限は UTF-8 の最大長から有界のバイト列を作り、文字の途中では切りません。ローカルモデルの構造を保つ JSON 圧縮は従来どおりです。保存処理には完全な結果を渡し、この表示処理で DB の内容を切り詰めません。

スクリーンショットの計画用要約は画面範囲と対象だけを解析し、画像本体は読み飛ばします。画像取得は MIME と画像データだけを解析し、未エスケープの文字列を借用してから必要な所有文字列へ一度だけ変換します。重複フィールドなど従来の形式は全体の解析へ戻して扱います。過去のツール名と JSON も履歴から借用し、結果を除外する前に大きい JSON 文字列を複製しません。擬似呼び出しを含まない文章は文字列変換中も借用します。

回帰テストは旧方式の JSON 出力との比較で、再帰的な除外項目、擬似呼び出しの置換、エスケープ、中文・日本語・emoji、全ての切断位置、上限ぴったりの場合を確認します。画像は未エスケープと復号が必要な文字列、空・不正な画像、重複フィールドを比較します。手動ベンチマークは `cargo test --manifest-path src-tauri/Cargo.toml --lib benchmark_tool_result_preparation -- --ignored --nocapture` で、一時の大きなテキストと画像からプレビュー・メタデータ・画像を作る処理を交互に測ります。実際の GPU、画面描画、アプリ全体の CPU/RSS の測定ではありません。

### 認識中断時の発話保持

macOS と Windows の `capture_error` は `native_agent_submission::capture_failed` を共有します。`SharedState::fail_capture` は入力 ID を確認し、その入力の所有権の終了、確定段落と末尾の仮字幕の取り出し、旧要求の所有権の解除、Notice の予約を同じロックで行います。中断後の `idle` や重複するエラーは再び内容を提出できず、旧入力のエラーは新しい入力を変更しません。

認識できた内容がある場合は通常の音声提出と同じ保存枠・トランザクション・再試行バッファを使います。保存 worker を UI 操作より先に登録し、Notice の表示・閉じる操作・次の録音とは独立して履歴に保存します。正常な提出も保存 worker を登録してから浮動 UI を更新します。中断した入力の回復は保存だけを行い、プロバイダー、モデル、ツール、回答の要求を開始しません。認識内容が空の初期化エラーは会話を作成せず、元のエラーを表示します。

保存の成否は同じ Notice の lease が現在も有効なときだけ反映します。別の Notice、閉じたパネル、次の録音を遅い保存結果で上書きしません。保存失敗の発話は再試行バッファに残します。ここで保存できるのは、すでに認識した段落と最後の仮字幕であり、認識できなかった音声の復元ではありません。

`consume_all_speech` は保持済みの String を取り出して使い、段落だけ・仮字幕だけの場合は全文を再コピーしません。先頭・末尾の Unicode 空白は元のバッファ内で取り除き、中日文のつなぎ方を保ちます。回帰テストは元のバッファの再利用、Unicode 空白、中断後の全文保存、重複・旧入力の拒否、空の初期化エラー、次の録音を開始した間の実際の SQLite 保存と遅い Notice の拒否を確認します。実際の認識器を故障させたり、マイクや GPU を起動したりするテストではありません。

### 退出・再起動前の保存

`app_shutdown.rs` は退出要求を保持し、イベントループを動かしたまま背景で STT の停止と保存を待ちます。STT の正常終了はマイクの取得済み音声、VAD の末尾、待機中の確定デコードを処理してから完了を通知します。停止の一秒待機が時間切れになっても、保存済みとは扱わず待機を続けます。認識器が故障した場合の破棄は正常停止と区別します。新しいマイク開始は STT 状態の予約時にも退出フラグを確認し、停止要求と予約の間をすり抜けません。

`PendingPersistence` は入力を受け付けた時点で保存枠を予約し、保存 worker がその所有権を持ちます。ネイティブの保存 worker は async 継続の実行前に登録されるため、待機中の継続を破棄しても受け付けた発話の保存を止めません。STT の末尾の同期コールバックが保存枠を予約した後に新規受付を閉じ、保存の完了を非同期に待ちます。通常の Agent 入力も最初のメッセージ保存までの枠を持ち、プロバイダー・キーチェーン・AI 推論の完了は退出条件に含めません。退出中は新しい Agent 推論を開始しません。

会話の作成・共有選択・タイトル変更・削除と、受理済みの AI 設定操作・モデル前の設定準備、および汎用 cache の読み書きは、独立した三つの `background_queue.rs` のインスタンスとして退出条件に含めます。退出の開始時に受付を同期的に閉じ、受付と同じ Mutex で境界を決めます。境界の前に入った命令は応答の破棄や未 poll によって取り消さず、既存の順序で実行します。境界の後の命令はキューや worker に追加せず「終了中」のエラーを返します。AI 設定のディスク反映と成功通知が終了するまで待ち、個別の設定保存エラーはその IPC に返します。設定準備後のモデル通信は待ちません。退出を中止する場合は会話・AI 設定・cache の三つの受付を再開します。STT の末尾が予約する音声保存は別の入力保存枠を使うため、この時点でも全文を保持できます。

STT の停止後、入力保存と会話・AI 設定・cache の各キューの完了を `tokio::join!` で非同期に待ちます。キューの空状態だけでなく、実行中の命令とコミット後の通知登録も終了したことを確認してから進みます。通知待機は状態を確認する前に登録し、最後の worker が idle になった際にすべての待機元を起こすため、完了通知の取りこぼしを防ぎます。会話命令の DB エラーや捕捉した panic は個別の IPC 応答へ返し、後続の命令を実行します。これらを録音の保存失敗として扱ったり、自動再試行したりしません。録音保存の失敗で退出を取り消した場合は、通知を閉じた後に会話キューの受付も再開し、まだ処理中の命令があればその後ろへ新しい命令を追加します。

最後に LIVE の同じ保存ゲートで全文のキャッシュと必要な Markdown を保存し、30 秒の debounce 中の静かな末尾も残します。最終要約の処理がすでに完全な AI 前の記録を保存した段階では、その Markdown を途中経過の文で上書きしません。保存失敗や worker の中断は退出を中止し、エラーを通知します。通知後は新しい受付を再開し、次の退出で失敗したネイティブ発話を再試行します。

macOS の `applicationShouldTerminate:` は AppKit の直接終了を取り消し、通常の Tauri 退出要求へ変換します。全画面 Space を抜ける処理も同じ経路を使い、UI への送信が遅れても `process::exit` で保存を飛ばしません。取消後の旧全画面 watchdog は世代で拒否します。再起動要求は一度通常の、保留できる退出要求へ変換します。保存が完了してから Tauri の再起動を要求するため、`RESTART_EXIT_CODE` の `prevent_exit` が無効な仕様に保存待機を依存させません。

回帰テストは保存枠の受付と拒否、八つの待機元への完了通知、async 継続の破棄後の実際の SQLite 保存、停止の複数回の時間切れ、最後のコールバックが登録する保存、LIVE の静かな末尾、保存エラーと再試行、実行中の保存の重複拒否、八スレッドからの再試行、再起動意図・退出コードの保持、取消後の旧全画面完了の拒否を確認します。会話キューでは応答の破棄・エラー・panic 後の drain、八つの終了待機元、同時の受付と封鎖、退出取消後の受付再開と順序維持を検証します。単一 async スレッドと実際の SQLite 書き込みロックで、入力保存が済んでも会話変更とそのコミット通知が終わるまで最終保存を始めないことを確認します。三つの queue を順に解放する検証では、cache の完了前にも LIVE の最終保存へ進まないことを確認します。製品の CRUD 処理を使い、受付済みの作成・共有選択・名前変更・削除を応答未 poll のまま封鎖しても、再接続後の DB と削除対象の要求の無効化が一致することも確認します。AppKit の実際の終了・Space 遷移、Windows の実際の終了、実機マイク、電源断はこのテストで検証していません。

### ネイティブ Agent の共有イベント処理

macOS と Windows は `native_agent_events.rs` の型付き STT sink を共有します。確定行・仮字幕・状態・エラーは STT バックエンドから直接渡し、その後でフロントエンド向け JSON イベントを送ります。Tauri が他のイベントコールバックのロックを待ち、イベントを遅延配信しても、発話の保持と `idle` による提出は STT の完了より先に行います。STT JSON を内部でも再解析する四つの購読を除去します。

入力 ID・Listening モードの確認、発話の変更、表示の世代の予約は共通状態の同じロック内で行います。停止要求後も自分の確定行と末尾の仮字幕を保持し、表示要求は追加しません。状態を変更するのは現在の入力だけで、旧・欠落・空の入力 ID や別 caller を採用しません。STT のエラーは元のメッセージをその入力の Notice に反映します。

Agent の回答 token・done・error も `agent/stream.rs` から共通 sink へ直接渡します。通常の文字列も改行・引用符・絵文字を含む文字列もバックエンドから借用し、現在の回答バッファへの追加だけで保持します。内部の JSON 購読・再解析・解除ハンドルを除去し、フロントエンド向けの JSON 送信は従来の形式で残します。phase・plan・tool・think はネイティブの回答本文へ追加しません。

`SharedState::begin_stream` は表示の予約と同じロック内で会話 ID と新しい要求 UUID を持つ `StreamOwner` を作ります。音声提出の受付時に `submit_voice_turn` が同じ ID で Agent 要求を同期登録し、保存・モデル解決・履歴準備を一つの blocking job で順に行います。保存後や async 待機の初回 poll で登録し直さないため、遅い保存が後から受け付けた別の要求を置き換えません。置換・待機の破棄後も受け付けた全文は保存し、取消済みならモデル解決へ進みません。バックエンドの要求の有効性確認に加えて、ネイティブ側でも会話 ID・要求 ID・Processing モードを照合します。同じ会話の別要求から来る token・done・error、古い会話、空の要求 ID、重複する終端を拒否します。正常な終端は一度だけ所有権を消費し、新しい表示は古い表示要求の lease を無効にします。

他の画面の要求によって背景の音声要求が置き換わると、旧要求の公開終端は抑止されます。提出タスクの終了時に元の `StreamOwner` を確認し、その要求をまだ待っているカプセルだけに取消の Notice を返します。通常の done/error で既に完了した表示、同じ会話の新しい要求、Listening の発話を上書きしません。保存失敗や提出エラーも同じ要求だけに適用します。保存後のモデル準備は別 worker を起動せず、そのまま同じ要求・保存 receipt で進めます。

`scripts/benchmark-native-events.sh` は現在の共通状態・要求所有権・回答バッファの製品ソースとプロジェクトの Cargo ロックを使い、Value JSON 解析・借用 JSON 解析の比較用実装と直接の型付き処理を release ビルドで比較します。日本語、エスケープを含む Unicode、別要求の破棄を各 20,000 回、七回順序を入れ替えて計測し、時間とヒープ割当要求を別々に測ります。結果バッファは事前に容量を確保し、全文一致と別要求の未採用を各計測後に確認します。Mutex の取得・要求照合・回答追加を含み、フロントエンド向けの JSON 生成・イベント送信、UI、モデル、音声認識、アプリ全体の CPU/RSS、GPU の改善率は含みません。

回帰テストは直接渡す全文の借用と所有文字列の移動、旧・欠落・空の入力 ID、停止後の発話保持、同じ会話の異なる要求、八スレッドからの旧・現要求の回答、終端の一度だけの処理、バックエンドの要求置換で終端が抑止された場合の取消表示を確認します。製品の音声 admission を使い、保存前・初回 poll 前の登録、保存待機中の後続要求の優先、async 待機の破棄後の保存とモデル準備の抑止、保存 receipt の一度だけの消費、保存エラー・worker panic 後に準備を始めないことも確認します。実際の一時 SQLite 保存を遅らせ、表示の要求を入れ替えても受け付けた全文が一度だけ残り、旧回答・失敗が新要求を変更しないことも検証します。実機のマイク・OS の表示・WebKit GPU 障害を検証するテストではありません。

### ネイティブ音声ショートカットの遅延開始

macOS の長押し待機後と Windows のキー押下時は、STT 状態の読み取りと開始要求を blocking worker へ送ります。Windows のウィンドウメッセージ処理、macOS の Fn polling と async 実行スレッドで状態照会の lock を待ちません。待機中は native state lock を解放し、照会前と照会後に同じ押下の版・held 状態・表示の epoch を照合します。松鍵、設定無効化、パネルの閉鎖や新しい表示によって失効した成功・エラーを採用せず、キーリピートも追加 worker を作りません。開始直前の native → STT の予約では `Listening` と入力 ID に加えて `stop_requested` を確認し、照会後に松鍵された入力もマイクを予約しません。予約自体の native/LIVE → STT という lock 順序は保持します。`stt/reservation.rs` は所有者の lock 内で STT registry の `try_lock` を行い、混雑時は所有者の lock を解放してから registry の空きを待ちます。待機後は録音 ID・終了状態を毎回読み直し、有効な所有者と registry の両方を保持した時だけ一度予約します。待機を録音の非活動や成功へ偽装せず、取消・置換や従来の poison/使用中エラーを返します。

`stt/native_stop.rs` は空いている registry では従来どおり直ちに停止フラグを設定します。混雑時だけ blocking worker に元の入力 ID を渡し、native のキー離上・閉鎖コールバックを戻します。worker は現在の `native_agent` と同じ入力 ID の組だけを停止し、別の native/LIVE/Agent 入力を停止しません。返答ハンドルを破棄しても受け付けた停止は続き、最後の字幕を排出する所有権を即時消去せず、teardown の完了も待ちません。混雑後の lock エラーはログへ残します。

開始予約と停止の回帰テストは隔離した実際の STT registry/control、共通 native state、製品の LIVE 所有者検証を使います。STT が混雑している間にも native/LIVE lock が取れること、松鍵・取消・保存開始・置換後に一度も予約しないこと、正常な再試行で両 lock を保持し元の control と ID を予約すること、停止の即時処理と遅い停止の所有者照合、停止ハンドルの破棄、poison と既存エラーの維持を確認します。LIVE の registry は隔離した代替スロットで、保存やマイクを開始しません。

macOS の停止補助タイマーも STT 状態を blocking worker で読み、待機後にタイマーの版と入力 ID を再確認します。共通状態の回帰テストは lock が占有された模擬 STT、単一 async スレッド、松鍵・閉鎖・置換・新しい表示、旧エラー、既存の native 入力と発話、予約直前の取消を確認します。実際のマイクや OS のキーイベント配信、GPU 白画面を検証するテストではありません。

### ネイティブ浮動パネルのアニメーション

macOS と Windows の字幕・ネイティブ Agent の UI 読み取り・アニメーションは `MainThreadAnimation` を共有します。UI スレッドへの要求は `oneshot` の非同期応答で待ち、同期 `recv()` で Tokio の実行スレッドを占有しません。幅・高さ・透明度、入力インジケーター・処理中の点・グラデーションの各ループは、一つのフレームの実行完了を待ってから次を生成します。主スレッドが遅いときも、各アニメーションが未実行フレームを際限なく追加しません。

要求前と主スレッド上の実行時に世代を確認し、古いアニメーションの最後のフレームも UI を上書きしません。待機タスクが破棄された場合も、まだ実行していない UI 操作を飛ばします。主スレッドへの送信失敗・要求の破棄ではアニメーションを終了します。初期の幅・透明度の読み取りでパネルが存在しない場合も終了し、画面座標をゼロで代用しません。

macOS の字幕と Agent のサイズ更新は `NSPanel.setFrame:display:` の `display` を `false` にし、フレームごとの即時再描画要求を除去します。ウィンドウの位置・サイズを変更した後、ビューの寸法・角丸を一つの明示的な Core Animation トランザクションで更新します。`macos_layer_transaction.rs` が暗黙のアニメーションを抑止し、テーマ変更も同じ処理を使います。手動で生成するフレームに別の補間を重ねず、字幕の幅がすでに目標値なら同じレイアウトを書き直しません。

トランザクションの終了は Rust のスコープガードで保証します。ネストした更新や Rust の unwind が外側の設定を残したまま戻り、後続の UI 処理に「暗黙のアニメーション無効」「持続時間ゼロ」が漏れません。回帰テストは実際の QuartzCore API のネスト・設定復元・unwind 後の次の更新を確認します。パネルやマイクを起動せずに実行し、AppKit の実際の表示や GPU の負荷削減率を測定した結果とは区別します。

字幕の文字更新と自動非表示の幅・透明度の予約は主スレッドで行います。遅れて実行される旧非表示が新しい字幕のアニメーションを取り消すことを防ぎます。閉じた字幕の文字更新と、閉じる要求の後に再度開いたパネルへの旧破棄も世代で拒否します。Agent の通常終了はフェードの実際の完了を待ってパネルを除去し、固定の待機時間で完了を推測しません。即時終了はフェードも無効化します。

Rust の回帰テストは一つの非同期実行スレッドと代替 UI キューを使い、主スレッド待機中に他のタスクが動くこと、未完了フレームが一つだけであること、旧フレーム・旧非表示・破棄された待機元の副作用を拒否すること、送信失敗と要求の破棄で待機が終わることを検証します。実際の AppKit 描画や WebKit GPU 障害を再現するテストではありません。

## 回帰チェック

```sh
npm test
npx tsc --noEmit
npm run build
cd src-tauri
cargo test --lib --no-fail-fast
cargo test --lib db::cache::tests::benchmark_unchanged_cache_sync -- --ignored --nocapture
# リポジトリのルートに戻って DSP を最適化コンパイルで測定
cd ..
scripts/benchmark-stt-resampler.sh
# macOS の Swift タスク登録だけを検証（モデルを起動しない）
scripts/test-apple-task-registry.sh
```

SQLite のベンチマークは一時 DB の 16 個の 1 MiB キャッシュを 100 回確認し、従来の全 JSON 読み込みと版だけの確認を比較します。値は環境によって変わるため、速度をテストの合否条件にはしません。実際の STT/GPU 負荷を再現するベンチマークではありません。

2026-10-07 の macOS ローカル測定では、100 回の合計が全 JSON 読み込み 544.17 ms、メタデータ確認 9.34 ms でした。この測定は未変更キャッシュの SQLite 読み取りに限定され、アプリ全体の速度を表しません。

## セッション管理

- セッション有効期限の自動検証
- 期限切れ時の自動再ログインフロー
- セキュアなクレデンシャル保存（macOS: Keychain / Windows: Credential Store）
