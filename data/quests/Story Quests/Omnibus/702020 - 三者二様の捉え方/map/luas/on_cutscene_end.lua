if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 702020)
        unlock_quest(sender, 702030)
        move_lobby(sender)
    end
end
