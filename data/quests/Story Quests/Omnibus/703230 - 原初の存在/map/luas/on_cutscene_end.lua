if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 703230)
        unlock_quest(sender, 703240)
        story_reward(sender, "", false)
        move_lobby(sender)
    end
end
