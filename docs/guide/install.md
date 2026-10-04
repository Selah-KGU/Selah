# インストール

## ダウンロード

[最新リリース](https://github.com/Selah-KGU/Selah/releases/latest) から、お使いの OS に合ったインストーラーをダウンロードしてください。

### macOS

1. `.dmg` ファイルをダウンロード
2. ダウンロードした `.dmg` を開き、`Selah.app` を `Applications` フォルダにドラッグ
3. 初回起動時に「開発元が確認できない」と表示される場合は、**システム設定 > プライバシーとセキュリティ** から「このまま開く」を選択

::: tip
macOS 11 Big Sur 以降が必要です。Apple Silicon (M1/M2/M3/M4) と Intel の両方に対応しています。
:::

### Windows

1. `.exe` (NSIS インストーラー) をダウンロード
2. インストーラーを実行し、指示に従ってインストール

::: tip
Windows 10 または Windows 11 が必要です。WebView2 ランタイムが自動的にインストールされます。
:::

## 初回セットアップ

1. Selah を起動すると、ログイン画面が表示されます
2. 関西学院大学の SSO アカウント（利用者 ID とパスワード）でログインします
3. ログインが完了すると、KWIC・Luna・メールのデータが自動的に取得されます

## AI 機能のセットアップ（任意）

AI 機能（Selah Agent・履修分析・学習計画・通知サマリー）はサイドバーの **設定 → AI 設定** から構成できます。macOS では Apple Intelligence が使える端末で端末内実行でき、クラウド API も利用できます。Windows ではクラウド API のみです。

### Apple Intelligence（API キー不要、macOS のみ）

アプリ自体は macOS 11 以降で起動します。端末内 AI は別に次の条件が必要です。

- macOS 26 以降
- Apple Intelligence がオンで、システムモデルの準備が完了していること

Windows ではローカル AI を利用できません。クラウド API を使用してください。

1. **AI 設定** の「推論方法」で **Apple Intelligence** を選択
2. モデルのダウンロードは不要です
3. 推論は端末内のシステムモデルで完結します

### クラウド AI

1. **AI 設定** の「プロバイダ」で **OpenAI** または **Gemini** を選択
2. **OpenAI API Key** または **Google Gemini API Key** を入力
3. 使用するモデルを選択して保存

::: info
API キーはローカルのキーチェーン（macOS）またはクレデンシャルストア（Windows）に安全に保存されます。
:::

## ビルド（開発者向け）

ソースからビルドする場合：

```bash
# 依存関係のインストール
npm install

# 開発モードで起動
npm run tauri dev

# リリースビルド
npm run tauri build
```

### 前提条件

- Node.js 20+
- Rust 1.80.0+
- [Tauri 2 の前提条件](https://tauri.app/start/prerequisites/)
