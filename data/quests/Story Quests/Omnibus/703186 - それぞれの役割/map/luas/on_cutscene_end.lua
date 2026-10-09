if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 703186)
        unlock_quest(sender, 703190)
        move_lobby(sender)
    end
end
