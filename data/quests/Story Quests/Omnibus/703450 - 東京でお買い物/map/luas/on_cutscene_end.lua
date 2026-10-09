if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 703450)
        unlock_quest(sender, 703460)
        move_lobby(sender)
    end
end
