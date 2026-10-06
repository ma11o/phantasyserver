if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 700953)
        unlock_quest(sender, 700954)
        move_lobby(sender)
    end
end
