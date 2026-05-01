# Selah アプリ更新リリース手順

Selah のアプリ内更新は Tauri v2 updater と GitHub Releases の `latest.json` で配信します。通常の CI ビルドは Release を作成しません。配信用の draft Release は `.github/workflows/release.yml` だけで作ります。

## バージョン番号ルール

形式は `MAJOR.MINOR.PATCH` の semver で、`PATCH = (UTC 年 - 2020) * 1000 + 通日` を CI が自動計算します。例: 2026-05-01 (UTC) → patch = `6 * 1000 + 121 = 6121` → `1.0.6121`。

- リポジトリにコミットする version は `MAJOR.MINOR.0` 固定（例: `1.0.0`）。先頭 2 セグメントだけが事実上の入力で、3 セグメント目の `0` は semver/Cargo/npm が 3 セグメントを要求するための占位です。CI は build job の workspace でだけ patch を上書きしてからビルドするので、仓库本体や git tree は変わりません（draft Release の成果物だけが日付付き版番号になります）。
- `MAJOR.MINOR` を上げたいときは `package.json` の `1.0.0` を `1.1.0` などに編集してコミットするか、`Release` workflow を手動実行する際の入力 `major_minor` で渡します。
- patch は年単位で必ず増えるので Tauri updater の semver 比較が壊れません。MSIX の各セグメント上限 (65535) も 2085 まで保ちます。
- ローカルで版番号を再計算したい場合は `node scripts/apply-version.mjs` で確認、`node scripts/apply-version.mjs $(node scripts/apply-version.mjs)` で全ファイルに反映できます。

## リリース前チェック

1. GitHub Actions secrets に `TAURI_SIGNING_PRIVATE_KEY` を設定する。
2. macOS の公開ビルドでは Developer ID / notarization 用 secrets も設定する。
3. `MAJOR.MINOR` を更新したい場合のみ `package.json` を編集してコミットする。それ以外、tag や 3 つの version ファイルを手で揃える必要はありません。

## 公開方法

1. `Release` workflow を手動実行する。`major_minor` を空にすると `package.json` 由来、明示すればその値が使われ、CI が `MAJOR.MINOR.<patch>` を組み立てて全 version ファイルを上書きしてからビルドする。
2. `release-macos` が macOS updater bundle と `.sig` を作る。
3. `release-windows` は macOS の後に実行され、Windows updater bundle と `.sig` を追加し、既存の `latest.json` に Windows platform を merge する。
4. `verify-updater-release` が draft Release を GitHub API で再取得し、`latest.json` と platform signature を検証する。
5. draft の installer を試用して問題なければ、GitHub の Release 画面でその draft を Publish する。Publish だけなら再ビルドは不要です。

## 必須 Release assets

- `latest.json`
- `Selah_universal.app.tar.gz`
- macOS updater signature: `*.app.tar.gz.sig`
- Windows installer: `Selah_<version>_x64-setup.exe`
- Windows updater signature: `*-setup.exe.sig`

`latest.json` には少なくとも以下の platform が必要です。

- `darwin-aarch64`
- `darwin-x86_64`
- `darwin-aarch64-app`
- `darwin-x86_64-app`
- `windows-x86_64`
- `windows-x86_64-nsis`

## よくある事故

- `Build` workflow の CI artifact は公開配信に使いません。配信用 draft は必ず `Release` workflow で作ります。
- `TAURI_SIGNING_PRIVATE_KEY` が空だと署名が作られず、Tauri Action は `latest.json` を skip することがあります。
- macOS と Windows の Release job を並列にすると `latest.json` の更新が競合します。Windows job は macOS job の後に実行します。
- draft の間は公開 `latest/download/latest.json` では確認できません。workflow は GitHub API 経由で draft asset を検証します。

## Store 版との分離

- Mac App Store 版は `VITE_SELAH_DISTRIBUTION_CHANNEL=appstore` と `--no-default-features --features stt-shared,store-build --config tauri.appstore.conf.json` でビルドします。この組み合わせでは updater plugin を登録せず、フロントエンドも store 用 stub に差し替え、UI は Mac App Store 管理の更新表示に切り替えます。
- Microsoft Store 版は MSIX / Store 管理更新に寄せます。`.github/workflows/msstore.yml` は `VITE_SELAH_DISTRIBUTION_CHANNEL=msstore` と `--no-default-features --features stt-shared,store-build --config tauri.msstore.conf.json` で self-updater を外した Windows ビルドを作り、`scripts/package-msix.ps1` で MSIX を生成します。UI は Microsoft Store 管理の更新表示に切り替わり、アプリ内更新ボタンは出しません。
- GitHub Releases の draft は直配布版だけの更新フィードです。Store 版を更新する場合は各 Store の申請フローで新しいビルドを提出します。
