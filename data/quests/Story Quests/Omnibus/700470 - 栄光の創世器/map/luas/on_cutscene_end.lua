if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 700470)
        unlock_quest(sender, 700480)
        move_lobby(sender)
    end
end
