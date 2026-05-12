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
|  | AI Client (local llama-cpp-2 /     |  |
|  |            OpenAI / Gemini)        |  |
|  | STT (sherpa-onnx + SenseVoice)     |  |
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

## SSO 認証連携

Selah は内蔵 WebView を用いた SSO セッション共有方式で認証を処理します。

1. 内蔵 WebView (WKWebView / WebView2) で関学 SSO のログイン画面を表示
2. ユーザーが SSO でログイン
3. WebView の認証セッションをネイティブ HTTP クライアント (`reqwest`) と共有
4. KG-Course・Luna・KWIC の 3 系統を一度のログインで認証

## データフロー

```
SSO Login
    |
    v
SSO Session --> reqwest HTTP Client
    |                    |
    +-----> KWIC API     |
    +-----> Luna API     |
    +-----> Mail API     |
                         v
                   HTML Parsing / JSON Parse
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
