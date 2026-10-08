# 20020 (生成物。`tools/gen_quest.py 20020` で作る。手で直さない)

- 入力: free-field-catalog.md の行 (stage family [510, 511])、雛形 `200030 - Test Quest`、`tools/quest_overrides/fields.csv` の行と `20020.json` (有れば)。
- `quest_obj.id` = 1202 (難易度側も同じ)。`quest_type` = Expedition。他の `unk*` と `difficulties` の中身は雛形のまま。
- ゾーン: campship (150) + ['campship_down=510', 'area2=511']。`args`・着地座標は docs/stage-spawn.csv (T25)。
- 対応表と変更理由: docs/findings/quest.md の「T24」。
