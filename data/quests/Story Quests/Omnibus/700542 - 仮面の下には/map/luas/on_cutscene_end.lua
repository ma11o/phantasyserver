if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 700542)
        unlock_quest(sender, 700550)
        move_lobby(sender)
    end
end
