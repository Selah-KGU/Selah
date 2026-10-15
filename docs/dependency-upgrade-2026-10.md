# 2026 年 10 月の依存関係更新

2026-10-15 に npm と crates.io の安定版を確認し、直接依存と両方のロックファイルを更新しました。開発環境と CI は Node.js 24 LTS / Rust 1.99.0 に統一しています。

## 主なバージョン

| 依存関係 | 更新前の指定 | 更新後の指定 |
| --- | --- | --- |
| Rust 最低バージョン | 1.93.0 | 1.99.0 |
| Svelte | 5.55.1 | 5.57.2 |
| Vite | 8.0.3 | 8.3.4 |
| TypeScript | 6.0.2 | 7.0.2 |
| Tauri API / Rust | 2.10.1 / 2.10.3 | 2.12.4 |
| Tauri CLI | 2.10.1 | 2.12.3 |
| Tauri updater | 2.10.1 / 2 | 2.14.0 |
| reqwest | 0.12 | 0.13.5 |
| cookie_store / reqwest_cookie_store | 0.21 / 0.8 | 0.22.1 / 0.10.0 |
| rusqlite | 0.32 | 0.40.2 |
| scraper | 0.22 | 0.27.0 |
| lopdf / pdfium-render | 0.35 / 0.8 | 0.45.0 / 0.9.4 |
| zip / quick-xml | 2 / 0.38 | 9.0.3 / 0.42.0 |
| CPAL | 0.15.3 | 0.18.2 |
| sherpa-onnx | 1.12.39 | =1.13.8 |
| rand / sha2 / aes-gcm | 0.8 / 0.10 / 0.10 | 0.10.3 / 0.11.1 / 0.11.1 |
| jsonwebtoken | 9.3 | 11.1.0 |
| Windows keyring / WebView2 | 3 / 0.38 | 4.2.0 / 0.39.1 |

その他の Tauri plugin、Serde、Tokio、画像・圧縮・macOS/Windows bindings も確認・更新しました。全指定は `package.json` と `src-tauri/Cargo.toml` を参照してください。

## 互換性の判断

- `@types/node` は Node.js 24 に合わせて 24.19.2 を使用します。
- VitePress は安定版の 1.6.4 を維持し、その配下の Vite を 6.4.4 以降へ override しています。これは既存の Vue plugin の対応範囲内で、旧 Vite / esbuild の advisory を解消するためです。アプリ本体の Vite は 8.3.4 です。
- Windows bindings は `windows` 0.62.2、`windows-core` 0.62.2、`windows-collections` 0.3.2 を WebView2 0.39.1 と組み合わせます。`windows-core` 単体を 0.100 系へ更新すると COM 型が一致しません。
- Windows ファイル共有の WinRT 非同期操作は、`windows-future` 0.3 の同期待機 API に合わせて `.get()` から `.join()` へ移行しました。
- reqwest の `form` / `query` を明示的に有効化し、native TLS、文字コード、HTTP/2、システム proxy を維持します。
- `jsonwebtoken` は `aws_lc_rs` backend を明示指定します。代替の `rust_crypto` backend が導入する `rsa` の未修正 advisory（RUSTSEC-2023-0071）を避けます。
- SHA-256 の出力は `hex::encode` に移行し、既存のアカウントディレクトリ名やキャッシュ ID の小文字 hex 形式を維持します。
- AES-GCM の保存形式は維持します。旧 aes-gcm 0.10.3 が生成した固定の v1 / v2 暗号文を、新版で復号する回帰テストを追加しています。nonce と master key は引き続き OS の乱数源から生成します。
- CPAL のサンプルレート、stream config、error 型と、quick-xml / zip / pdfium-render の API 変更を反映しました。
- Tauri 2.12 の remote-origin ACL 検査に合わせ、MockRuntime の IPC テストは固定の Windows 用 URL ではなく、実際のテスト window の URL を使います。
- sherpa-onnx の Rust FFI と配布 runtime は 1.13.8 に固定し、macOS / Windows x64 / ARM64 の配布 archive の SHA-256 を確認しました。macOS の ONNX Runtime の配布名変更も bundling 設定へ反映しています。

## 検証

- フロントエンド: 監査ルールの回帰テスト 8 件を含む 529 テスト、TypeScript の型チェック、アプリと VitePress の production build が成功。
- Rust: 全体の 1,109 テストと、追加の旧 vault 復号テスト 1 件が成功。手動 benchmark / モデル fixture を必要とする 22 件は既定どおり ignored。
- 通常構成と `--no-default-features --features stt-shared,store-build` の全 target のコンパイルチェックが成功。
- native runtime: macOS universal sherpa-onnx の生成・バージョン取得、および同梱 PDFium の実際のページ描画が成功。
- `npm audit`: 既知の脆弱性 0 件。
- `npm run audit:rust`: vulnerability / 未承認 warning 0 件。Tauri の Linux GTK3 専用依存に対する二件の例外は、以下の到達性チェック付きで管理します。
- Windows クロスチェックは macOS 上の Windows SDK 不在（`assert.h` がない）で停止しました。Windows x64 / ARM64 の最終ビルドは既存の Windows CI で確認する必要があります。
- Windows ファイル共有モジュールは Tauri の接続部分だけを stub にした独立 crate で x64 / ARM64 の型チェックに成功。旧 `.get()` では CI と同じ E0599 が再現し、`.join()` で解消することを確認しました。

署名、配布ストアへの提出、実マイクによる録音はこの更新の検証に含みません。

## RustSec のプラットフォーム限定例外

`src-tauri/.cargo/audit.toml` は、次の二件だけを現在の配布範囲に対する例外とします。該当する上流コード自体を修正したという意味ではありません。

| Advisory | 対象 | 例外の根拠 |
| --- | --- | --- |
| [RUSTSEC-2024-0370](https://rustsec.org/advisories/RUSTSEC-2024-0370.html) | proc-macro-error 1.0.4 | GLib / GTK3 の macro からの間接依存。現在の macOS / Windows 四 target では build dependency としても使用されません。 |
| [RUSTSEC-2024-0429](https://rustsec.org/advisories/RUSTSEC-2024-0429.html) | glib 0.18.5 | Linux GTK3 の間接依存。修正版は 0.20 以降ですが、Tauri の GTK3 0.18 系とはそのまま置き換えられません。現在の配布 target には含まれません。 |

`scripts/audit-rust-dependencies.mjs` は全機能を有効にした macOS ARM64 / x64 と Windows ARM64 / x64 の normal・build・dev 依存を確認してから `cargo audit --deny warnings` を実行します。次の場合は監査を失敗させ、例外の再評価または削除を要求します。

- 対象 package がいずれかの配布 target で到達可能になる。
- package の版・取得元が変わる、重複する、または上流更新により不要になる。
- CI に Linux などの未確認の配布 target を追加する。
- ignore を追加・削除・重複する、または依存グラフを取得できない。

追加した `Dependency audit` workflow は、該当ファイルを変更する PR / main push と手動実行でこのチェックを行います。新しい advisory や yanked package は例外へ自動追加せず、監査を失敗させます。
