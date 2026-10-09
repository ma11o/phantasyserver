if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 703510)
        unlock_quest(sender, 703520)
        move_lobby(sender)
    end
end
