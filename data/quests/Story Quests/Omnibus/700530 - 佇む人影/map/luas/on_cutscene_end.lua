if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 700530)
        unlock_quest(sender, 700541)
        move_lobby(sender)
    end
end
