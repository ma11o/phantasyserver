if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 702470)
        unlock_quest(sender, 702480)
        move_lobby(sender)
    end
end
