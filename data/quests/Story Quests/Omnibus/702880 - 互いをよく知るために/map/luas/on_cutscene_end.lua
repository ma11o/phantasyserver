if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 702880)
        unlock_quest(sender, 702890)
        move_lobby(sender)
    end
end
