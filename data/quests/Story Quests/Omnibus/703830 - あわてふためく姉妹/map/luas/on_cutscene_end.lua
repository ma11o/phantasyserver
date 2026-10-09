if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 703830)
        unlock_quest(sender, 703840)
        move_lobby(sender)
    end
end
