if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 701690)
        unlock_quest(sender, 701700)
        move_lobby(sender)
    end
end
