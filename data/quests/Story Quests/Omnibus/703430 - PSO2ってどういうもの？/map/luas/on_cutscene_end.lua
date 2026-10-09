if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 703430)
        unlock_quest(sender, 703440)
        move_lobby(sender)
    end
end
