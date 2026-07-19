# 再配布チェックリスト

この文書はKarukanの独立したmacOS配布物を作る際の確認事項です。法的助言ではなく、
配布時点のライセンス本文と各配布元の条件が優先されます。

## コード本体

- Karukan本体は`MIT OR Apache-2.0`です。
- バイナリ配布ではMIT Licenseを選択し、`LICENSE-MIT`と原著作者の表示を
  `Karukan.app/Contents/Resources/`へ同梱します。
- fork元と主な変更内容をREADMEまたは配布ページに表示します。
- 製品名、アイコン、Bundle IDは著作権ライセンスとは別に検討します。独立製品として
  公開する場合は、混同を避ける名称と自身が管理するreverse-DNS形式のBundle IDを
  使用します。

## 同梱データと依存物

- `make licenses`で`THIRD_PARTY_CARGO_LICENSES.html`を更新します。
- `THIRD_PARTY_LICENSES`、`THIRD_PARTY_CARGO_LICENSES.html`、
  `MODEL_LICENSES.md`がアプリへ入っていることを確認します。
- GGUFモデルをアプリへ同梱する場合は、モデルカードとCC BY-SA 4.0の表示・継承条件を
  再確認します。現在の標準構成ではモデルをHugging Faceから別途取得します。
- `dict.bin`を配布する場合は、`dictionary-sources.toml`でソースとSHA-256を固定し、
  `DICTIONARY_LICENSES.md`を同じアーカイブへ含めます。

## macOS配布

- 自身が管理するBundle IDへ変更します。
- `CFBundleShortVersionString`と`CFBundleVersion`を更新します。
- Developer IDで署名し、Appleのnotary serviceで公証します。
- Apple SiliconとIntelの両方を対象にする場合は`make build-universal`を使います。
- クリーン環境でインストール、入力ソース追加、モデル・辞書取得、変換、更新、
  アンインストールを確認します。

## 最終確認

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cd karukan-macos
make test
make bundle
codesign --verify --deep --strict out/Karukan.app
```
