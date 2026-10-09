if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 700400)
        unlock_quest(sender, 700420)
        story_reward(sender, "", false)
        move_lobby(sender)
    end
end
