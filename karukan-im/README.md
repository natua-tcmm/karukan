# karukan-im

fcitx5（Linux）および macOS Swift フロントエンドで共有される日本語IMEエンジン。ローマ字→ひらがな変換、GPT-2ベースのニューラルかな漢字変換、文節修正学習、システム辞書を提供します。

フロントエンドのインストール手順:
- Linux (fcitx5): [karukan-fcitx5](../karukan-fcitx5/README.md)
- macOS: [karukan-macos](../karukan-macos/README.md)

## Features

- ニューラルかな漢字変換（llama.cppによるGGUF推論）
- 変換学習（明示修正した文節を左右の文脈付きで記憶）
- 辞書ラティスによる複数文節変換・文節境界調整
- 日本語・英数字の混合入力（Shift切り替え）
- Surrounding Textによる文脈を考慮した変換
- システム辞書・ユーザー辞書による候補補完

> [!NOTE]
> モデル推論だけでは語彙が限られるため、システム辞書の併用を強く推奨します。システム辞書はIMEに同梱されていないため、別途インストールが必要です。詳しくは [Dictionary](#dictionary) を参照してください。

## Key Bindings

### ひらがな入力モード

| キー | 動作 |
|------|------|
| 文字キー | ローマ字入力 → ひらがな変換 |
| Space | かな漢字変換を開始 |
| Tab / ↓ | 入力中に表示されている候補を選択（連打で次候補） |
| Enter | 通常は入力中文字列を確定。Tab / ↓ で候補選択中は選択候補を確定 |
| Escape | IMEでは何もしない（変換中のみキャンセル） |
| Backspace | 1文字削除 |
| Delete | カーソル位置の文字を削除 |
| ← → | カーソル移動 |
| Home / End | カーソルを先頭 / 末尾に移動 |
| F6 / F7 / F8 | 入力中文字列をひらがな / 全角カタカナ / 半角カタカナで即確定 |
| Ctrl+Space | 全角スペースを入力 |

### 変換モード

| キー | 動作 |
|------|------|
| Space / Tab / ↓ | 次の候補 |
| ↑ | 前の候補 |
| ← / → | 変換対象の文節を移動 |
| Shift+← / Shift+→ | 現在の文節と次の文節の境界を縮小 / 拡張 |
| 1-9 / 候補クリック | 候補を現在の文節へ適用（変換は継続） |
| Enter | 全文節をまとめて確定 |
| Escape | 変換をキャンセル（ひらがなに戻る） |
| 文字キー | 全文節を確定して新しい入力を開始 |

### モード切り替え

| キー | 動作 |
|------|------|
| Shift+英字 | 英数字モードに切り替え + 大文字入力 |
| Right Super | 英数字/カタカナ → ひらがなモードに復帰 |

### 英数字モード

英数字モードでは文字がローマ字変換されず、そのまま入力されます。Enterで確定するとひらがなモードに復帰します。日本語と英語を混ぜて入力し、Spaceで変換するとひらがな部分のみ変換されます。

例: `わたしはLinuxが` → 変換 → `私はLinuxが`

## Configuration

設定ファイル: `~/.config/karukan-im/config.toml`（macOS: `~/Library/Application Support/com.karukan.karukan-im/config.toml`）

```toml
[conversion]
live_conversion = true          # ライブ変換を起動時に有効化（既定ON）
live_num_candidates = 3         # ライブ変換中に生成するモデル候補数
composing_chunk_len = 30        # ライブ変換で1回のモデル変換が扱う読みの最大文字数（= 1キーあたりレイテンシの上限）
num_candidates = 9              # 変換候補数（Space押下時）
n_threads = 4                   # 推論スレッド数（0 = 全コア使用）
model = "jinen-v1-small-q5"     # ライブ変換・Space変換で使うモデルID
use_context = true              # Surrounding Textを変換に使用する
max_context_length = 10         # コンテキストの最大文字数
dict_path = "/path/to/dict.bin" # システム辞書パス（省略時はデータディレクトリの dict.bin。[Dictionary](#dictionary) 参照）

[learning]
enabled = true                 # 変換学習の有効/無効
max_entries = 10000            # 学習エントリの最大数
```

> [!NOTE]
> 上記は主要な設定項目の抜粋です。全項目の正確な既定値と説明は [`config/default.toml`](config/default.toml) を参照してください（各設定行に日本語コメント付き）。

### Live Conversion

入力と同時にかな漢字変換の結果をプリエディットへリアルタイム表示します（Spaceを押さずに変換が進む）。既定では `live_conversion = true` で有効です。

長文入力でも1キーあたりのレイテンシを一定に保つため、変換中のバッファを内部で最大 `composing_chunk_len` 文字（既定30）のチャンクに分割し、編集した箇所のチャンクだけを再変換します。チャンクは内部的な分割で、ユーザーには連続した1つのプリエディットとして見えます。記号・数字の連続は日本語とは別チャンクに分けてそのまま通すため、`123456` のような並びが変換で崩れることはありません。

### Model Selection

ライブ変換とSpace変換は、どちらも `model` で指定した1つのモデルを使用します。標準は `jinen-v1-small-q5` です。メモリ使用量を抑えたい場合は、小型モデルを明示的に選択できます。

```toml
[conversion]
model = "jinen-v1-xsmall-q5"
```

Light/Adaptiveモードは廃止されています。旧設定の `strategy`、`light_model`、`short_input_threshold`、`beam_width`、`max_latency_ms` は互換性のため読み飛ばされ、変換には常に `model` で指定した単一モデルが使われます。

### Performance Tuning

CPU高負荷時（Rustビルド中など）にかな漢字変換が遅くなる場合は、`n_threads` を小さくするとレスポンスが改善します。

### Dictionary

辞書の構築・管理については [karukan-cli の README](../karukan-cli/README.md) を参照してください。

#### System Dictionary

double-array trieベースのシステム辞書で、モデル推論に加えて辞書からの変換候補を提供します。

- デフォルトパス: `~/.local/share/karukan-im/dict.bin`（macOS: `~/Library/Application Support/com.karukan.karukan-im/dict.bin`）
- `dict_path` で任意のパスを指定可能
- ファイルが存在しない場合は辞書なしで動作

ビルド済みの辞書を以下からダウンロードして配置できます:

```bash
# Linux
wget https://github.com/togatoga/karukan/releases/download/v0.1.0/dict.tgz
tar xzf dict.tgz
mkdir -p ~/.local/share/karukan-im
cp dict.bin ~/.local/share/karukan-im/

# macOS
curl -LO https://github.com/togatoga/karukan/releases/download/v0.1.0/dict.tgz
tar xzf dict.tgz
mkdir -p ~/Library/"Application Support"/com.karukan.karukan-im
cp dict.bin ~/Library/"Application Support"/com.karukan.karukan-im/
```

自分でビルドする場合は [karukan-cli の README](../karukan-cli/README.md) を参照してください。

#### User Dictionary

ユーザー辞書ディレクトリにファイルを配置すると、ユーザー辞書として読み込まれます。

- デフォルトパス: `~/.local/share/karukan-im/user_dicts/`（macOS: `~/Library/Application Support/com.karukan.karukan-im/user_dicts/`）
- ディレクトリ内のファイルはすべて自動で読み込み（KRKNバイナリ・Mozc TSV を自動判定）
- ディレクトリが存在しない場合はユーザー辞書なしで動作

変換候補の基本優先順位:

1. 📝 文節修正学習（現在の読みと左右ヒントが一致する場合）
2. 👤 ユーザー辞書
3. 🤖 モデル推論・モデルと辞書の合成候補
4. 📚 システム辞書ラティス
5. ひらがな / カタカナ
6. 🔄 Rewriter（半角カタカナ・英字全角半角・記号バリアント）

### Learning Cache

学習は過去文章の補完ではなく、ユーザーが明示的に変更した変換の訂正記憶として動作します。候補移動や数字・クリックで表記を変更した文節だけを記録し、読みと文脈が一致したときに優先表示します。

- 保存先: `~/.local/share/karukan-im/segment_learning.tsv`（macOS: `~/Library/Application Support/com.karukan.karukan-im/segment_learning.tsv`）
- 記録内容: 読み、確定表記、左右の隣接文字、頻度、最終使用日時
- 例: 「あと、」で「後」を「あと」に直すと、次回も右側が「、」のときに「あと」を優先
- 全文候補を変更した場合も、初期候補との差分を安全に対応付けられた文節だけを記録
- 初期候補やライブ変換をそのまま確定した内容は記録しない
- 前方一致による過去文章の予測候補は表示しない
- IME切り替え・ウィンドウ切り替え時に自動保存（commit のたびには保存しない）
- `[learning] enabled = false` で無効化可能
- 学習履歴を削除するには: `rm ~/.local/share/karukan-im/segment_learning.tsv`

以前のバージョンが作成した `learning.tsv` は読み込みません。自動削除もしないため、不要であれば手動で削除できます。

### JSON-RPC Protocol

macOSフロントエンドと`karukan-imserver`の通信プロトコルはv2です。`select_candidate`は表示ページ内の候補を**現在の文節へ適用するだけ**で、全文確定は`commit`またはEnterで行います。Rust側の`PROTOCOL_VERSION`とSwift側の`supportedEngineProtocolVersion`は同時に更新してください。
