if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 701265)
        unlock_quest(sender, 701270)
        move_lobby(sender)
    end
end
