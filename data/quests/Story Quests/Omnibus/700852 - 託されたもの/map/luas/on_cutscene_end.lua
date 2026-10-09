if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 700852)
        unlock_quest(sender, 700860)
        move_lobby(sender)
    end
end
