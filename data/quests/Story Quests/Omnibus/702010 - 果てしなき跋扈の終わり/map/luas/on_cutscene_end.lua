if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 702010)
        unlock_quest(sender, 702020)
        move_lobby(sender)
    end
end
