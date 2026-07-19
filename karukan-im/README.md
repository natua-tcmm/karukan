# karukan-im

macOSのSwift/InputMethodKitフロントエンドから利用するRust製IMEエンジンです。
状態管理、ローマ字変換、かな漢字変換、辞書、文節修正学習を担当します。

## 構成

- `InputMethodEngine`: Empty → Composing → Conversionの状態機械
- `karukan-imserver`: 改行区切りJSON-RPC 2.0をstdin/stdoutで処理するサーバー
- `karukan-engine`: ローマ字変換、辞書、llama.cppによるGGUF推論

Swift側は`karukan-macos/Sources/KarukanIME/EngineProtocol.swift`に同じプロトコル型を
定義しています。ワイヤ形式を変更する場合は、Rust側の
`src/server/protocol.rs`と同時に更新してください。

## ビルドとテスト

リポジトリのルートで実行します。

```bash
cargo build -p karukan-im --release
cargo test -p karukan-im
```

サーバーへ直接JSON-RPCを送ることもできます。

```bash
cargo run -p karukan-im --bin karukan-imserver
{"jsonrpc":"2.0","id":1,"method":"process_key","params":{"keysym":107}}
```

## 設定とデータ

保存先は`~/Library/Application Support/com.karukan.karukan-im/`です。

- `config.toml`: 設定
- `dict.bin`: システム辞書
- `user_dicts/`: ユーザー辞書
- `segment_learning.tsv`: 文節修正学習データ

設定例は`config/default.toml`、ユーザー辞書の形式は
[`docs/user-dictionary.md`](../docs/user-dictionary.md)を参照してください。

## ライセンス

ソースコードはMIT OR Apache-2.0です。配布に必要な第三者表示は、リポジトリルートの
`THIRD_PARTY_LICENSES`と`MODEL_LICENSES.md`を参照してください。
