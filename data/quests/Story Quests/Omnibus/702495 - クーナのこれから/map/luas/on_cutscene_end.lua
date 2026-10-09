if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 702495)
        unlock_quest(sender, 702500)
        move_lobby(sender)
    end
end
