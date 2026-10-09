if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 700250)
        unlock_quest(sender, 700270)
        story_reward(sender, "", false)
        move_lobby(sender)
    end
end
