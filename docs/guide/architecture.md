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
|  | AI Client (OpenAI / Gemini)        |  |
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

## Cookie Bridge 認証

Selah は独自の **Cookie Bridge** 方式で SSO 認証を処理します。

1. 内蔵 WebView (WKWebView / WebView2) で関学 SSO のログインフォームを表示
2. ユーザーが SSO でログイン
3. WebView から取得した Cookie を Rust 側の HTTP クライアント (`reqwest`) に橋渡し
4. KG-Course・Luna・KWIC の 3 系統を一度のログインで認証

## データフロー

```
SSO Login
    |
    v
Cookie Bridge --> reqwest HTTP Client
    |                    |
    +-----> KWIC API     |
    +-----> Luna API     |
    +-----> Mail API     |
                         v
                   HTML Scraping / JSON Parse
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

## セッション管理

- セッション有効期限の自動検証
- 期限切れ時の自動再ログインフロー
- セキュアなクレデンシャル保存（macOS: Keychain / Windows: Credential Store）
