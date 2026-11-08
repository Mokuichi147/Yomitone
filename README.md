# Yomitone

小説テキストを章ごとに読み込み、登場人物とセリフの話者を推測して読み上げるツールです。音声合成を有効にすると、章ごとの WAV ファイルを書き出します。

## はじめに

利用には、JSON Schema による構造化出力に対応した OpenAI API 互換の LLM サーバーが必要です。人物抽出と、LLM を使った話者推測に利用します。Jev を話者推測に使う場合も、人物抽出には LLM サーバーが必要です。

音声を書き出す場合は、OpenAI 互換の `/v1/audio/speech` または Wyoming に対応した TTS サーバーも必要です。ローカルサーバーなど認証が不要な場合、API キーは指定せずに使えます。

### CLI をインストールする

Rust 1.88 以上を用意し、リポジトリのルートで実行します。

```bash
cargo install --path crates/yomitone-cli
yomitone --help
```

### デスクトップアプリを起動する

```bash
cargo run --release -p yomitone-app
```

Linux でアプリをビルドする場合は GTK4 と libadwaita の開発用ライブラリが必要です。Ubuntu では次のコマンドでインストールできます。

```bash
sudo apt install libgtk-4-dev libadwaita-1-dev
```

## 入力ファイル

章ごとに分けた UTF-8 の `.txt` ファイルを、1つのフォルダに置いてください。

- ファイル名の自然順に処理します。登場人物と音声の割り当ては章をまたいで引き継ぎます。
- 1行を1文として扱います。行全体が `「……」` で囲まれた文は話者を推測し、それ以外はナレーターが読みます。

## デスクトップアプリ

「読み上げ」で入力フォルダや話者推測、音声出力を設定して開始します。「結果」では登場人物と音声の割り当てを確認できます。接続先や音声は左側の「設定」から変更できます。

設定は自動保存され、次回起動時に引き継がれます。API キーは設定ファイルに保存されません。「保存」を押すと、macOS ではキーチェーン、Windows では資格情報マネージャーに保存します。Linux では環境変数を使うか、起動ごとに入力してください。

保存したキーを読み込めない場合は、API キー欄の「読み込む」を押してください。環境変数は LLM と OpenAI 互換 TTS で `OPENAI_API_KEY`、Jev で `TYPESAFE_API_KEY` を使います。

## CLI の使い方

入力フォルダの登場人物と話者を推測し、結果を表示します。

```bash
yomitone --host localhost --port 1234 --model qwen3.8-flash-next-iq3_s novels/
```

Jev で話者を推測する場合は `--speaker-backend jev` を指定します。

```bash
yomitone --speaker-backend jev novels/
```

音声を書き出す場合は `--tts-output` と TTS サーバーの設定を指定します。

```bash
yomitone --tts-output out/ --tts-backend openai \
  --tts-url http://localhost:8880/v1 --tts-voices voice1,voice2 novels/
```

```bash
yomitone --tts-output out/ --tts-backend wyoming \
  --tts-url tcp://localhost:10200 novels/
```

OpenAI 互換 TTS では音声一覧を取得できないため、`--tts-voices` に音声名を指定してください。Wyoming では省略すると、`--tts-language` に合う音声をサーバーから取得します。

API キーは `--llm-api-key`、`--jev-api-key`、`--tts-api-key` または環境変数で指定できます。コマンドラインで指定したキーを優先します。認証不要のサーバーでは省略してください。

### 音声の割り当て

- `--tts-narrator-voice` を省略すると、音声一覧の先頭をナレーターに使います。
- 登場人物にはナレーター以外の音声を使用数が均等になるように割り当てます。同じ人物には後の章でも同じ音声を使います。
- `--tts-male-voices` と `--tts-female-voices` を指定すると、該当する性別の人物にはその音声を優先します。

## オプション

| オプション | 既定値 | 説明 |
| --- | --- | --- |
| `--host` / `--port` | `localhost` / `1234` | LLM サーバーのホストとポート |
| `--model` | `qwen3.8-flash-next-iq3_s` | LLM のモデル名 |
| `--llm-api-key` | `OPENAI_API_KEY` | LLM の API キー |
| `--speaker-backend` | `llm` | 話者推測に使うバックエンド (`llm` / `jev`) |
| `--jev-model` | `TYPESAFE_DEFAULT_MODEL` または `jev-latest` | Jev のモデル名 |
| `--jev-base-url` | `TYPESAFE_BASE_URL` または `https://api.typesafe.ai` | Jev API の URL |
| `--jev-api-key` | `TYPESAFE_API_KEY` | Jev の API キー |
| `--tts-output` | なし | 音声の出力先。指定したとき音声合成を行います |
| `--tts-backend` | `openai` | TTS の方式 (`openai` / `wyoming`) |
| `--tts-url` | OpenAI: `http://localhost:8880/v1` / Wyoming: `tcp://localhost:10200` | TTS サーバーの URL |
| `--tts-model` | `tts-1` | OpenAI 互換 TTS のモデル名 |
| `--tts-api-key` | `OPENAI_API_KEY` | OpenAI 互換 TTS の API キー |
| `--tts-language` | `ja` | Wyoming の音声を絞り込む言語。空欄で絞り込みません |
| `--tts-voices` | Wyoming: サーバーの一覧 | 使用する音声名 (カンマ区切り) |
| `--tts-narrator-voice` | 音声一覧の先頭 | ナレーターの音声 |
| `--tts-male-voices` / `--tts-female-voices` | なし | 性別ごとに優先する音声名 (カンマ区切り) |
| `--tts-pause` | `0.3` | 行間に入れる無音の秒数 (最大 60 秒) |
| `--tts-max-chars` | `200` | 1回の合成に送る最大文字数。`0` で分割しません |
| `--tts-timeout` | `60` | TTS サーバーへのリクエストのタイムアウト秒数 |

詳しい引数は `yomitone --help` でも確認できます。
