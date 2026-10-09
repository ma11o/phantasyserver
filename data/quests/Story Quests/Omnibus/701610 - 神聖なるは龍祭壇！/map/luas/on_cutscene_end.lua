if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 701610)
        unlock_quest(sender, 701620)
        move_lobby(sender)
    end
end
