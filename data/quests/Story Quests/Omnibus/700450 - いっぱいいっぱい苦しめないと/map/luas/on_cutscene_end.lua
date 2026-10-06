if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 700450)
        unlock_quest(sender, 700460)
        move_lobby(sender)
    end
end
