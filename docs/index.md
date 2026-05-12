---
layout: home

hero:
  name: Selah
  text: 新月の下で、知性を繋ぐ。
  tagline: 授業・課題・成績・お知らせ・メールをひとつにまとめる、関学生向けデスクトップアプリ
  image:
    src: /logo.png
    alt: Selah
  actions:
    - theme: brand
      text: ダウンロード
      link: https://github.com/Selah-KGU/Selah/releases/latest
    - theme: alt
      text: 機能を見る
      link: /guide/features
    - theme: alt
      text: GitHub
      link: https://github.com/Selah-KGU/Selah

features:
  - icon: "\U0001F3E0"
    title: ダッシュボード
    details: 現在・次の授業、お知らせ、締切間近の課題、天気情報をひと目で確認。
  - icon: "\U0001F4C5"
    title: 時間割
    details: KWIC と Luna のデータを統合したグリッド表示。Google カレンダーへの自動同期にも対応。
  - icon: "\U0001F4DD"
    title: TODO / 課題管理
    details: Luna の課題をまとめて管理。レポート提出や掲示板投稿もアプリから直接可能。
  - icon: "\U0001F4E7"
    title: メール
    details: 大学 Microsoft 365 メールの受信トレイ閲覧。ブラウザ不要。
  - icon: "\U0001F4CA"
    title: 成績照会
    details: 系列ごとの必要単位・修得単位をテーブルで確認。
  - icon: "\U0001F3A4"
    title: LIVE 講義文字起こし
    details: リアルタイム STT で発話を即時テキスト化し、AI が定期的に講義内容を要約。macOS では字幕オーバーレイにも対応。
  - icon: "\U0001F916"
    title: Selah Agent
    details: ローカル AI または OpenAI / Gemini による対話型エージェント。履修分析・学習計画・通知サマリーを生成し、音声入力にも対応。
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

.intro-gallery {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 20px;
  margin: 24px 0 40px;
}

.intro-card {
  display: block;
  overflow: hidden;
  border: 1px solid var(--vp-c-divider);
  border-radius: 24px;
  background: linear-gradient(180deg, var(--vp-c-bg-soft), var(--vp-c-bg));
  color: inherit;
  text-decoration: none !important;
  box-shadow: 0 18px 40px rgba(27, 45, 91, 0.08);
  transition: transform 0.2s ease, box-shadow 0.2s ease, border-color 0.2s ease;
}

.intro-card:hover {
  transform: translateY(-4px);
  border-color: var(--vp-c-brand-1);
  box-shadow: 0 24px 48px rgba(27, 45, 91, 0.14);
}

.intro-card img {
  display: block;
  width: 100%;
  aspect-ratio: 16 / 10;
  object-fit: cover;
  object-position: top center;
  background: #f5f1e6;
}

.intro-card-copy {
  padding: 18px 18px 20px;
}

.intro-card-copy h3 {
  margin: 0 0 8px;
  font-size: 1.08rem;
  border-bottom: none !important;
  padding-bottom: 0 !important;
}

.intro-card-copy p {
  margin: 0;
  color: var(--vp-c-text-2);
  line-height: 1.7;
}

@media (max-width: 960px) {
  .intro-gallery {
    grid-template-columns: 1fr;
  }
}
</style>

## Selah とは

Selah は関西学院大学の教務システム **KWIC**、学習管理システム **Luna (LMS)**、大学の **Microsoft 365 メール**を統合し、ネイティブデスクトップアプリとして提供するサードパーティクライアントです。

ユーザー自身の大学アカウントで SSO ログインし、自分のデータにアクセスします。ブラウザを開かずに、授業・課題・成績・お知らせ・メールをすばやく確認できます。

::: warning 注意
Selah は個人開発のサードパーティデスクトップクライアントであり、大学の公式アプリケーションではありません。
:::

## 画面で見る Selah

ホーム、時間割、シラバス検索は、Selah の中でも確認頻度が高い中心画面です。毎日の情報整理がどう変わるかを、まず 3 枚で見られるようにしました。

<div class="intro-gallery">
  <a class="intro-card" href="/p1.png" target="_blank" rel="noreferrer">
    <img src="/p1.png" alt="Selah のホーム画面">
    <div class="intro-card-copy">
      <h3>ホーム</h3>
      <p>今日の授業、お知らせ、締切、天気を 1 画面に集約。アプリを開いた瞬間に必要な情報へ届きます。</p>
    </div>
  </a>
  <a class="intro-card" href="/p2.png" target="_blank" rel="noreferrer">
    <img src="/p2.png" alt="Selah の時間割画面">
    <div class="intro-card-copy">
      <h3>時間割</h3>
      <p>週間グリッドで授業と試験を一覧表示。休講確認やカレンダー同期、AI 日程機能ともつながります。</p>
    </div>
  </a>
  <a class="intro-card" href="/p3.png" target="_blank" rel="noreferrer">
    <img src="/p3.png" alt="Selah のシラバス検索画面">
    <div class="intro-card-copy">
      <h3>シラバス検索</h3>
      <p>ログイン後の学部情報を自動で引き継ぎ、お気に入り登録した科目は時間割にも反映できます。</p>
    </div>
  </a>
</div>

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
| AI | ローカル (llama-cpp-2 + Qwen) / OpenAI / Google Gemini |
| 音声認識 | sherpa-onnx + SenseVoice (オンデバイス) |

---

[プライバシーポリシー](/privacy) | [利用規約](/terms)
