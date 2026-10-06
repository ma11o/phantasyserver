if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 702600)
        unlock_quest(sender, 702610)
        move_lobby(sender)
    end
end
