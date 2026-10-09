if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 701310)
        unlock_quest(sender, 701320)
        move_lobby(sender)
    end
end
