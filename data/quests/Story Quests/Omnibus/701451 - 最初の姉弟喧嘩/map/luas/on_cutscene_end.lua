if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 701451)
        unlock_quest(sender, 701460)
        move_lobby(sender)
    end
end
