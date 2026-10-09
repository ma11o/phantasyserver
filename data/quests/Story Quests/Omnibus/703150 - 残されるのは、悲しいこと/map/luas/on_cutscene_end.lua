if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 703150)
        unlock_quest(sender, 703160)
        story_reward(sender, "", false)
        move_lobby(sender)
    end
end
