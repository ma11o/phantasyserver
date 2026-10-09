if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 703560)
        unlock_quest(sender, 703580)
        move_lobby(sender)
    end
end
