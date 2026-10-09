if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 702955)
        unlock_quest(sender, 702970)
        move_lobby(sender)
    end
end
