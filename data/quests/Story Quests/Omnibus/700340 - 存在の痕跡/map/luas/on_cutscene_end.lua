if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 700340)
        unlock_quest(sender, 700350)
        story_reward(sender, "", false)
        move_lobby(sender)
    end
end
