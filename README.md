<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="128" height="128" alt="Selah">
</p>

<h1 align="center">Selah</h1>

<p align="center">
  新月の下で、知性を繋ぐ。すべての関学生に。
</p>

<p align="center">
  <a href="../../releases/latest"><img src="https://img.shields.io/github/v/release/mirai-mamori/Selah?label=download&style=flat-square" alt="Latest Release"></a>
  <img src="https://img.shields.io/badge/platform-macOS%2011%2B-blue?style=flat-square" alt="Platform">
  <img src="https://img.shields.io/github/license/mirai-mamori/Selah?style=flat-square" alt="License">
</p>

---

> **注意**: Selah は関西学院大学の**非公式**デスクトップクライアントです。大学との提携・承認はありません。個人プロジェクトとして開発されています。

Selah は関西学院大学の教務システム **KWIC** と学習管理システム **Luna (LMS)**、さらに大学の **Microsoft 365 メール**のデータを統合し、ネイティブデスクトップアプリとして提供します。ブラウザを開かずに、授業・課題・成績・お知らせ・メールをすばやく確認できます。

## 機能一覧

### ホーム

ダッシュボード画面。ログイン後に最初に表示されます。

- 時間帯に応じた挨拶メッセージ（朝・昼・夕・夜で変化）
- **NOW / NEXT** — 現在進行中または次の授業をカードで表示
- 直近のお知らせ（KWIC・Luna 統合、最新 3 件）
- スケジュール — 今日・明日の授業一覧をタイル表示
- 締切が近い課題（5 日以内）を緊急度別に色分け表示
- 天気情報の表示

### メール

- 大学の Microsoft 365 メールの受信トレイを表示
- メール本文のプレビュー・閲覧
- ページング対応

### 時間割

- KWIC の週間時間割をグリッド表示（月〜土、1〜7 限）
- Luna の時間割データとの統合表示
- 週単位のナビゲーション（前週 / 次週）
- 試験時間割の表示
- シラバスお気に入りを時間割上にオーバーレイ表示
- **AI 履修分析** — OpenAI / Gemini API を使い、成績・履修状況・シラバスを基に履修アドバイスレポートを生成（API キーは別途設定）

### TODO

- Luna (LMS) の課題・提出物を一覧表示
- 未提出 / 提出済のステータス管理
- 締切日の表示、期限超過の強調表示
- 課題の詳細をクリックで表示

### 成績照会

- 系列ごとの必要単位・履修単位・修得単位をテーブル表示
- 学生情報バー（氏名・学籍番号・学部・学科・年次）

### 履修登録

- 登録済み科目の一覧（曜日・時限・学期・授業名・教員・単位・状態）
- 単位数サマリーのカード表示
- 履修登録画面を別ウィンドウで表示

### シラバス検索

- 年度・学期・キャンパス・学部・曜日時限・キーワード・教員名・使用言語による検索
- 検索結果のテーブル表示
- お気に入り（ブックマーク）機能 — お気に入りに登録したシラバスは時間割にも表示可能

### お知らせ

- KWIC のお知らせと Luna の通知を統合表示
- KWIC / Luna のタブ切り替え
- 新着通知のネイティブ通知（macOS 通知センター連携）
- Luna の通知はクリックで詳細画面を表示

### 変更情報

- 休講情報・補講情報・教室変更をタブで切り替え表示
- 自分の学部の情報を優先表示

### その他

- **SSO 認証** — 関学の SSO (Single Sign-On) を経由してログイン。KWIC・Luna・Microsoft 365 にシームレスにアクセス
- **セッション自動管理** — セッション有効期限の自動検証（3 分ごと）、期限切れ時の自動再ログイン
- **バックグラウンドポーリング** — 時間割・お知らせ・TODO・メールなどを定期的に取得し、キャッシュを自動更新
- **トレイステータス** — メニューバーに現在の授業・次の授業・未提出課題などをサイクル表示
- **施設予約** — 大学の施設予約ページへのクイックアクセス
- **macOS ネイティブ** — タイトルバーオーバーレイ、トラフィックライト対応、ネイティブ通知

## 技術スタック

| レイヤー | 技術 |
|---------|------|
| フレームワーク | [Tauri 2](https://tauri.app/) |
| フロントエンド | [Svelte 5](https://svelte.dev/) + TypeScript |
| バックエンド | Rust (reqwest, scraper, tokio) |
| ビルドツール | [Vite](https://vitejs.dev/) |
| パッケージ | DMG / .app (macOS universal binary) |

## ビルド

### 前提条件

- Node.js 20+
- Rust stable (aarch64-apple-darwin, x86_64-apple-darwin)
- macOS 11.0+

### 手順

```bash
# 依存関係のインストール
npm install

# 開発サーバー起動
npm run tauri dev

# プロダクションビルド (macOS universal)
npm run tauri build -- --target universal-apple-darwin
```

ビルド成果物は `src-tauri/target/release/bundle/` に出力されます。

## ダウンロード

[Releases](../../releases) ページから最新の DMG インストーラーをダウンロードできます。

## 免責事項

- 本アプリは関西学院大学の公式アプリケーションではありません。大学の教務システムに対する非公式クライアントであり、大学との提携・承認はありません。
- 認証情報はローカルマシン上でのみ処理され、第三者サーバーには送信されません。
- 大学側のシステム変更により、予告なく動作しなくなる可能性があります。
- 利用は自己責任でお願いします。

## ライセンス

[MIT License](LICENSE)
