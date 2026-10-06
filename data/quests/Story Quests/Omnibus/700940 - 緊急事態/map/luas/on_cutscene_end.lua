if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 700940)
        unlock_quest(sender, 700951)
        move_lobby(sender)
    end
end
