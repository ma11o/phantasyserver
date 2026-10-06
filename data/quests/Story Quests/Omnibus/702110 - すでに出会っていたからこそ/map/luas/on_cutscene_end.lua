if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 702110)
        unlock_quest(sender, 702120)
        move_lobby(sender)
    end
end
