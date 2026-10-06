if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 702330)
        unlock_quest(sender, 702340)
        move_lobby(sender)
    end
end
