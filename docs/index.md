---
layout: home

hero:
  name: Selah
  text: 新月の下で、知性を繋ぐ。
  tagline: すべての関学生のための統合キャンパスデスクトップアプリ
  image:
    src: /logo.png
    alt: Selah
  actions:
    - theme: brand
      text: ダウンロード
      link: https://github.com/mirai-mamori/Selah/releases/latest
    - theme: alt
      text: 機能を見る
      link: /guide/features
    - theme: alt
      text: GitHub
      link: https://github.com/mirai-mamori/Selah

features:
  - icon: "\U0001F3E0"
    title: ダッシュボード
    details: 現在・次の授業、お知らせ、締切間近の課題、天気情報をひと目で確認。
  - icon: "\U0001F4C5"
    title: 時間割
    details: KWIC と Luna のデータを統合したグリッド表示。Apple / Google カレンダーへの自動同期にも対応。
  - icon: "\U0001F4DD"
    title: TODO / 課題管理
    details: Luna の課題をまとめて管理。レポート提出や掲示板投稿もアプリから直接可能。
  - icon: "\U0001F4E7"
    title: メール
    details: 大学 Microsoft 365 メールの受信トレイ閲覧。ブラウザ不要。
  - icon: "\U0001F4CA"
    title: 成績照会
    details: 系列ごとの必要単位・修得単位をテーブルで確認。
  - icon: "\U0001F916"
    title: AI アシスタント
    details: OpenAI / Gemini による履修分析・学習計画・通知サマリーを生成。
---

<style>
:root {
  --vp-c-brand-1: #1B2D5B;
  --vp-c-brand-2: #243A6E;
  --vp-c-brand-3: #1B2D5B;
  --vp-c-brand-soft: rgba(27, 45, 91, 0.14);
  --vp-home-hero-name-color: #1B2D5B;
  --vp-home-hero-image-background-image: radial-gradient(ellipse 80% 60% at 0% 20%, #FFD43Baa, transparent), radial-gradient(ellipse 70% 50% at 90% 10%, #1B2D5B66, transparent), radial-gradient(ellipse 60% 80% at 50% 90%, #FFD43B55, transparent), radial-gradient(ellipse 90% 70% at 100% 70%, #1B2D5Baa, transparent);
  --vp-home-hero-image-filter: blur(56px);
  --vp-button-brand-bg: #1B2D5B;
  --vp-button-brand-hover-bg: #243A6E;
  --vp-button-brand-active-bg: #142247;
}

.dark {
  --vp-c-brand-1: #FFD43B;
  --vp-c-brand-2: #FFE066;
  --vp-c-brand-3: #FFD43B;
  --vp-c-brand-soft: rgba(255, 212, 59, 0.14);
  --vp-home-hero-name-color: #FFD43B;
  --vp-home-hero-image-background-image: radial-gradient(ellipse 80% 60% at 0% 20%, #FFD43Baa, transparent), radial-gradient(ellipse 70% 50% at 85% 5%, #4A6FA566, transparent), radial-gradient(ellipse 60% 80% at 45% 95%, #FFD43B55, transparent), radial-gradient(ellipse 90% 70% at 100% 65%, #4A6FA5aa, transparent);
  --vp-button-brand-bg: #FFD43B;
  --vp-button-brand-hover-bg: #FFE066;
  --vp-button-brand-active-bg: #FCC419;
  --vp-button-brand-text: #1B2D5B;
}
</style>

## Selah とは

Selah は関西学院大学の教務システム **KWIC**、学習管理システム **Luna (LMS)**、大学の **Microsoft 365 メール**を統合し、ネイティブデスクトップアプリとして提供するサードパーティクライアントです。

ユーザー自身の大学アカウントで SSO ログインし、自分のデータにアクセスします。ブラウザを開かずに、授業・課題・成績・お知らせ・メールをすばやく確認できます。

::: warning 注意
Selah は個人開発のサードパーティデスクトップクライアントであり、大学の公式アプリケーションではありません。
:::

### 対応プラットフォーム

| OS | バージョン |
|---|---|
| macOS | 11 Big Sur 以降 |
| Windows | 10 / 11 |

### 技術スタック

| レイヤー | 技術 |
|---|---|
| フレームワーク | Tauri 2 |
| フロントエンド | Svelte 5 + TypeScript |
| バックエンド | Rust |
| ローカル DB | SQLite (WAL) |
| AI | OpenAI / Google Gemini API |

---

[プライバシーポリシー](/privacy) | [利用規約](/terms)
