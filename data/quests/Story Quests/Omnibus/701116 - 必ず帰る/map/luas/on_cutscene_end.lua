if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 701116)
        unlock_quest(sender, 701117)
        move_lobby(sender)
    end
end
