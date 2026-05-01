# Selah アプリ更新リリース手順

Selah のアプリ内更新は Tauri v2 updater と GitHub Releases の `latest.json` で配信します。`main` への commit で `.github/workflows/build.yml` が走り、同じバージョンの draft Release を作成または更新します。動作確認後、GitHub の Release 画面でその draft を手動 Publish します。

## バージョン番号ルール

形式は `MAJOR.MINOR.PATCH` の semver で、`PATCH` は `1..999` の 3 桁以内に収めます。CI は UTC 月を上旬・中旬・下旬の 3 期間に分け、各期間に 5 個の patch slot を割り当てます。式は `patch_floor = ((month - 1) * 3 + period) * 5 + 1` です。例えば 5 月上旬は `61`、5 月中旬は `66`、5 月下旬は `71` が日付由来の下限になります。

- リポジトリにコミットする version は `MAJOR.MINOR.0` 固定（例: `1.0.0`）。先頭 2 セグメントだけが事実上の入力で、3 セグメント目の `0` は semver/Cargo/npm が 3 セグメントを要求するための占位です。CI は build job の workspace でだけ patch を上書きしてからビルドするので、仓库本体や git tree は変わりません。
- `MAJOR.MINOR` を上げたいときは `package.json` の `1.0.0` を `1.1.0` などに編集してコミットします。手動実行の兜底として `Release` workflow の入力 `major_minor` も残しています。
- patch は同じ `MAJOR.MINOR` 内で必ず増えるので Tauri updater の semver 比較が壊れません。日付由来の下限は 12 月下旬でも `176` なので、増加ペースはかなり緩やかです。
- 最高バージョンが draft の場合は同じ draft を更新します。最高バージョンが Publish 済みの場合は、`max(月旬 slot, 最高 patch + 1)` で次の patch を選びます。
- 1 期間内に 5 回以上 Publish した場合は、次の期間の slot を前借りする形で `+1` し続けます。`999` までに到達したら `MAJOR.MINOR` を上げます。
- ローカルで月旬 slot 由来の版番号だけ確認したい場合は `node scripts/resolve-release-version.mjs`、GitHub Releases を見て次の実配布版を解決する場合は CI 内で `node scripts/resolve-release-version.mjs --github` を使います。`node scripts/apply-version.mjs <version>` で全ファイルに反映できます。

## リリース前チェック

1. GitHub Actions secrets に `TAURI_SIGNING_PRIVATE_KEY` を設定する。
2. macOS の公開ビルドでは Developer ID / notarization 用 secrets も設定する。
3. `MAJOR.MINOR` を更新したい場合のみ `package.json` を編集してコミットする。それ以外、3 つの version ファイルを手で揃える必要はありません。

## 公開方法

1. `main` に commit する。`Build` workflow が `MAJOR.MINOR.<patch>` を解決します。
2. 同じ tag の draft Release が既にあれば削除して作り直します。既に Publish 済みなら resolver が次の patch を選びます。
3. `build-macos` が macOS updater bundle と `.sig` を作って draft Release を作成します。
4. `build-windows` は macOS の後に実行され、Windows updater bundle と `.sig` を追加し、既存の `latest.json` に Windows platform を merge します。
5. `verify-updater-draft` が draft Release を GitHub API で再取得し、`latest.json` と platform signature を検証します。
6. draft の installer を試用して問題なければ、GitHub の Release 画面でその draft を Publish します。Publish だけなら再ビルドは不要です。
7. 例外的に手動で作る場合だけ、`Release` workflow を実行します。

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

- `Build` workflow は CI artifact に加えて配信用 draft Release も作ります。公開は手動 Publish だけで行います。
- `TAURI_SIGNING_PRIVATE_KEY` が空だと署名が作られず、Tauri Action は `latest.json` を skip することがあります。
- macOS と Windows の Release job を並列にすると `latest.json` の更新が競合します。Windows job は macOS job の後に実行します。
- draft の間は公開 `latest/download/latest.json` では確認できません。workflow は GitHub API 経由で draft asset を検証します。

## Store 版との分離

- Mac App Store 版は `VITE_SELAH_DISTRIBUTION_CHANNEL=appstore` と `--no-default-features --features stt-shared,store-build --config tauri.appstore.conf.json` でビルドします。この組み合わせでは updater plugin を登録せず、フロントエンドも store 用 stub に差し替え、UI は Mac App Store 管理の更新表示に切り替えます。
- Microsoft Store 版は MSIX / Store 管理更新に寄せます。`.github/workflows/msstore.yml` は `VITE_SELAH_DISTRIBUTION_CHANNEL=msstore` と `--no-default-features --features stt-shared,store-build --config tauri.msstore.conf.json` で self-updater を外した Windows ビルドを作り、`scripts/package-msix.ps1` で MSIX を生成します。UI は Microsoft Store 管理の更新表示に切り替わり、アプリ内更新ボタンは出しません。
- GitHub Releases の draft は直配布版だけの更新フィードです。Store 版を更新する場合は各 Store の申請フローで新しいビルドを提出します。
