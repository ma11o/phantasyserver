if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 702890)
        unlock_quest(sender, 702900)
        move_lobby(sender)
    end
end
