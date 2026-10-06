if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 700951)
        unlock_quest(sender, 700952)
        move_lobby(sender)
    end
end
