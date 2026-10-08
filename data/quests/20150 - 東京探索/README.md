# 20150 (生成物。`tools/gen_quest.py 20150` で作る。手で直さない)

- 入力: free-field-catalog.md の行 (stage family [810, 890])、雛形 `200030 - Test Quest`、`tools/quest_overrides/fields.csv` の行と `20150.json` (有れば)。
- `quest_obj.id` = 1215 (難易度側も同じ)。`quest_type` = Expedition。他の `unk*` と `difficulties` の中身は雛形のまま。
- ゾーン: campship (150) + ['campship_down=810', 'area2=890']。`args`・着地座標は docs/stage-spawn.csv (T25)。
- 対応表と変更理由: docs/findings/quest.md の「T24」。
