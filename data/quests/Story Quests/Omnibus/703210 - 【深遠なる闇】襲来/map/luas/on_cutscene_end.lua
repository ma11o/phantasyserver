if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 703210)
        unlock_quest(sender, 703212)
        move_lobby(sender)
    end
end
