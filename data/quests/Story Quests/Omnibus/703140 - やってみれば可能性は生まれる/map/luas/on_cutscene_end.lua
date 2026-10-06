if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 703140)
        unlock_quest(sender, 703150)
        move_lobby(sender)
    end
end
