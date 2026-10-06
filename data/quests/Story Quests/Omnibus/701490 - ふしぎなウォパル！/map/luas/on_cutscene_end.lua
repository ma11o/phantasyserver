if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 701490)
        unlock_quest(sender, 701500)
        move_lobby(sender)
    end
end
