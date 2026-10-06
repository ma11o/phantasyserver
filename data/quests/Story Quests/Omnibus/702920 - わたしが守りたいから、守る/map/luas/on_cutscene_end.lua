if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 702920)
        unlock_quest(sender, 702930)
        move_lobby(sender)
    end
end
