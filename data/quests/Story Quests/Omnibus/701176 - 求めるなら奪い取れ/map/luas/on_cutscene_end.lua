if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 701176)
        unlock_quest(sender, 701177)
        move_lobby(sender)
    end
end
