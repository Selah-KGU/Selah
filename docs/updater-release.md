# Selah アプリ更新リリース手順

Selah のアプリ内更新は Tauri v2 updater と GitHub Releases の `latest.json` で配信します。通常の CI ビルドは Release を作成しません。配信用の draft Release は `.github/workflows/release.yml` だけで作ります。

## リリース前チェック

1. `package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml` の version を同じ値にする。
2. GitHub Actions secrets に `TAURI_SIGNING_PRIVATE_KEY` を設定する。
3. macOS の公開ビルドでは Developer ID / notarization 用 secrets も設定する。
4. `vX.Y.Z` タグを version と一致させる。

## 公開方法

1. version と一致する tag を push する、または `Release` workflow を手動実行する。
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
- Microsoft Store に EXE/MSI として提出する場合は、Store は既存ユーザーへ自動更新を配りません。Microsoft の案内ではアプリ内更新も許容されるため、現状は直配布版と同じ updater を使えます。MSIX など Store 管理更新に切り替える場合は `VITE_SELAH_DISTRIBUTION_CHANNEL=msstore` を使って UI 文言を分けます。
