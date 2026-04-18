# 概要

**Selah** は関西学院大学の教務システム KWIC、学習管理システム Luna (LMS)、大学の Microsoft 365 メールを統合し、ネイティブデスクトップアプリとして提供するサードパーティクライアントです。ユーザー自身の大学アカウントで SSO ログインし、自分のデータにアクセスします。

## なぜ Selah？

関学生の日常では、授業情報の確認のために KWIC、課題の提出のために Luna、メールの確認のために Office 365 と複数のブラウザタブを行き来する必要があります。Selah はこれらを 1 つのアプリに統合し、よりスムーズな学習体験を提供します。

## 主な特徴

- **ワンクリック SSO ログイン** -- 関学の SSO 認証に対応。一度のログインで KWIC・Luna・メールすべてにアクセス
- **高速起動** -- SQLite によるローカルキャッシュと SWR 方式で、ネットワーク待ちなしに即座にデータを表示
- **ネイティブ通知** -- 新着お知らせ・課題をデスクトップ通知でお届け
- **AI アシスト** -- OpenAI / Gemini API を活用した履修分析・学習計画生成
- **オフライン対応** -- キャッシュされたデータでネット不通時も閲覧可能
- **クロスプラットフォーム** -- macOS と Windows に対応

## 技術スタック

Selah は [Tauri 2](https://tauri.app/) をベースに構築されています。

| コンポーネント | 技術 |
|---|---|
| フレームワーク | Tauri 2 (Rust + WebView) |
| フロントエンド | Svelte 5 + TypeScript + Vite |
| バックエンド | Rust (reqwest, html5ever, rusqlite) |
| ローカル DB | SQLite (WAL モード) |
| AI 統合 | OpenAI API / Google Gemini API |
| デスクトップ統合 | macOS: WKWebView, Apple Calendar / Windows: WebView2, NSIS |

## ライセンス

Selah は [MIT License](https://github.com/mirai-mamori/Selah/blob/main/LICENSE) の下で公開されています。
