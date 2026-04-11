# nitrocop-config: EnforcedStyle: relative_to_receiver
records.uniq { |el| el[:profile_id] }
       .map do |message|
         SomeJob.perform_later(message[:id])
       end
