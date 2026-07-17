# 辞書ライセンス確認表

`dict.bin`はKarukan本体とは別の第三者データを含み得ます。配布時は、実際に使用したソースを`dictionary-sources.toml`へ固定し、**各取得物に同梱されたライセンスと配布元の最新利用条件を優先**してください。この文書は確認手順であり、法的助言やライセンス許諾そのものではありません。

## 対応ソース

| ソース | 主な用途 | 配布前の確認事項 |
|---|---|---|
| Mozc | 一般語彙・記号 | Google由来部分はBSD 3-Clauseです。ただし、MozcのOSS辞書には複数の第三者データが含まれるため、使用したファイル固有のREADMEと著作権表示を確認します。 |
| SudachiDict | 一般語彙・固有名詞 | 対象リリースに同梱された`LICENSE`、`LEGAL`等を確認し、必要な表示を配布物へ含めます。 |
| JMdict / JMnedict | 一般語彙・人名・地名・駅名 | EDRDGの現行ライセンスを確認し、帰属表示、継承条件、データ更新日を記録します。 |
| 日本郵便 郵便番号データ | 都道府県・市区町村・町域 | 公式ダウンロードページと利用条件を確認します。郵便番号APIの規約とCSV配布データの条件を混同しません。 |
| 国土地理院 地名情報 | 地名・自然地物 | 国土地理院コンテンツ利用規約、出典表示、第三者権利、測量成果に該当する場合の手続を確認します。 |
| SKK辞書 | 地名・駅名等 | 辞書ごとに由来と条件が異なります。対象ファイルの配布ページ、`COPYING`、ヘッダーを確認し、一括して同じライセンスと推定しません。 |

## 配布チェック

1. `dictionary-sources.toml`のURLを不変なバージョンへ固定し、取得物のSHA-256を記録します。
2. ソース名、バージョンまたは更新日、配布元URL、ライセンス識別子、加工内容を記録します。
3. 帰属表示、ライセンス本文、NOTICE、継承条件を`DICTIONARY_LICENSES.md`へまとめます。
4. 再配布が不明なソースは`dict.bin`へ含めず、利用条件を確認してから有効化します。
5. `dict.tgz`に`dict.bin`、ソースマニフェスト、ライセンス文書、チェックサムを同梱します。

## 公式参照先

- [Mozc](https://github.com/google/mozc)
- [SudachiDict](https://github.com/WorksApplications/SudachiDict)
- [EDRDG Licence](https://www.edrdg.org/edrdg/licence.html)
- [日本郵便 郵便番号データダウンロード](https://www.post.japanpost.jp/zipcode/dl/)
- [国土地理院コンテンツ利用規約](https://www.gsi.go.jp/kikakuchousei/kikakuchousei40182.html)
- [SKK dictionary files](https://skk-dev.github.io/dict/)

