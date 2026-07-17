# システム辞書のリリース手順

配布用`dict.tgz`は、バージョン固定したソースから再生成でき、収録データの由来を利用者が確認できる状態にします。

## 1. ソースを固定する

`dictionary-sources.toml`へ、使用する各ソースのバージョン付きURL、SHA-256、ライセンス識別子、優先度、ファイル名、アーカイブ形式を記入します。`sources = []`のままリリースしてはいけません。

```bash
cargo run --release --bin karukan-dict-sources -- verify dictionary-sources.toml
cargo run --release --bin karukan-dict-sources -- fetch \
  dictionary-sources.toml --cache-dir target/dictionary-sources
```

## 2. 正規化して辞書を生成する

取得したアーカイブを展開後、一般語彙は`karukan-dict-import`、住所・地名・駅名は`karukan-dict-geography`へ渡します。複数ソースを一つの辞書へまとめる場合は、正規化JSONLを結合してから重複排除するか、リリース用の統合処理で`(reading, surface)`単位に統合します。

```bash
cargo run --release --bin karukan-dict-import -- \
  --sudachi target/dictionary-sources/system_core.csv \
  --jmdict target/dictionary-sources/JMdict_e.xml \
  --output target/general.jsonl \
  --binary target/general.bin

cargo run --release --bin karukan-dict-geography -- \
  --japan-post target/dictionary-sources/utf_ken_all.csv \
  --jmnedict target/dictionary-sources/JMnedict.xml \
  --output target/geographic.jsonl \
  --binary target/geographic.bin
```

一般語彙と地理語彙を統合し、配布用`dict.bin`を生成します。

```bash
cargo run --release --bin karukan-dict-import -- \
  --normalized target/general.jsonl \
  --normalized target/geographic.jsonl \
  --output target/system-dictionary.jsonl \
  --binary target/dict.bin
```

## 3. 内容を検査する

```bash
cargo run --release --bin karukan-dict -- view target/dict.bin --query 十中八九 --surface
cargo run --release --bin karukan-dict -- view target/dict.bin --query とうきょう --prefix
cargo test -p karukan-engine --lib
```

一般語彙、慣用句、都道府県、市区町村、町域、駅名、自然地物をサンプル検索し、文字化け、読みの未正規化、極端な候補数、禁止データの混入がないことを確認します。

## 4. ライセンスを確定する

[辞書ライセンス確認表](dictionary-licenses.md)に従い、実際の取得物に同梱された条件を確認します。必要な帰属表示とライセンス本文を`docs/dictionary-licenses.md`へ反映してからパッケージ化します。

## 5. パッケージ化する

```bash
scripts/package-dictionary.sh target/dict.bin target/dict.tgz
tar tzf target/dict.tgz
```

アーカイブには次を含めます。

- `dict.bin`
- `dictionary-sources.toml`
- `DICTIONARY_LICENSES.md`
- `SHA256SUMS`

GitHub Releaseへ`dict.tgz`を添付すると、macOSの`make install`は`releases/latest/download/dict.tgz`から取得します。既存の利用者が再現できるよう、辞書の生成日、収録ソースのバージョン、`dict.bin`のSHA-256をリリースノートにも記載します。
